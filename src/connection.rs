use std::fmt;
use std::net::Ipv4Addr;
use std::rc::Rc;

use dbus_nm::DBusNetworkManager;
use errors::*;

use device::{get_active_connection_devices, Device};
use ssid::{AsSsidSlice, Ssid};
use wifi::{AccessPoint, AccessPointCredentials};

const WIFI_TIMEOUT: u64 = 240;

#[derive(Clone)]
pub struct Connection {
    dbus_manager: Rc<DBusNetworkManager>,
    path: String,
    settings: ConnectionSettings,
}

impl Connection {
    fn init(dbus_manager: &Rc<DBusNetworkManager>, path: &str) -> Result<Self> {
        let settings = dbus_manager.get_connection_settings(path)?;

        Ok(Connection {
            dbus_manager: Rc::clone(dbus_manager),
            path: path.to_string(),
            settings,
        })
    }

    pub fn settings(&self) -> &ConnectionSettings {
        &self.settings
    }

    pub fn get_state(&self) -> Result<ConnectionState> {
        let active_path_option = get_connection_active_path(&self.dbus_manager, &self.path)?;

        if let Some(active_path) = active_path_option {
            let state = self.dbus_manager.get_connection_state(&active_path)?;

            Ok(state)
        } else {
            Ok(ConnectionState::Deactivated)
        }
    }

    pub fn delete(&self) -> Result<()> {
        self.dbus_manager.delete_connection(&self.path)
    }

    /// Activate a Network Manager connection.
    pub fn activate(&self) -> Result<ConnectionState> {
        let state = self.get_state()?;

        match state {
            ConnectionState::Activated => Ok(ConnectionState::Activated),
            ConnectionState::Activating => wait(
                self,
                &ConnectionState::Activated,
                WIFI_TIMEOUT,
            ),
            ConnectionState::Unknown => bail!(ErrorKind::NetworkManager(
                "Unable to get connection state".into()
            )),
            _ => {
                self.dbus_manager.activate_connection(&self.path)?;

                wait(
                    self,
                    &ConnectionState::Activated,
                    WIFI_TIMEOUT,
                )
            }
        }
    }

    /// Deactivates a Network Manager connection.
    pub fn deactivate(&self) -> Result<ConnectionState> {
        let state = self.get_state()?;

        match state {
            ConnectionState::Deactivated => Ok(ConnectionState::Deactivated),
            ConnectionState::Deactivating => wait(
                self,
                &ConnectionState::Deactivated,
                WIFI_TIMEOUT,
            ),
            ConnectionState::Unknown => bail!(ErrorKind::NetworkManager(
                "Unable to get connection state".into()
            )),
            _ => {
                let active_path_option =
                    get_connection_active_path(&self.dbus_manager, &self.path)?;

                if let Some(active_path) = active_path_option {
                    self.dbus_manager.deactivate_connection(&active_path)?;

                    wait(
                        self,
                        &ConnectionState::Deactivated,
                        WIFI_TIMEOUT,
                    )
                } else {
                    Ok(ConnectionState::Deactivated)
                }
            }
        }
    }

    pub fn get_devices(&self) -> Result<Vec<Device>> {
        let active_path_option = get_connection_active_path(&self.dbus_manager, &self.path)?;

        if let Some(active_path) = active_path_option {
            get_active_connection_devices(&self.dbus_manager, &active_path)
        } else {
            Ok(vec![])
        }
    }
}

impl Ord for Connection {
    fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
        i32::from(self).cmp(&i32::from(other))
    }
}

impl PartialOrd for Connection {
    fn partial_cmp(&self, other: &Self) -> Option<::std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Connection {
    fn eq(&self, other: &Connection) -> bool {
        i32::from(self) == i32::from(other)
    }
}

impl Eq for Connection {}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Connection {{ path: {:?}, settings: {:?} }}",
            self.path, self.settings
        )
    }
}

impl<'a> From<&'a Connection> for i32 {
    fn from(val: &Connection) -> i32 {
        val.clone()
            .path
            .rsplit('/')
            .next()
            .unwrap()
            .parse::<i32>()
            .unwrap()
    }
}

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub struct ConnectionSettings {
    pub kind: String, // `type` is a reserved word, so we are using `kind` instead
    pub id: String,
    pub uuid: String,
    pub ssid: Ssid,
    pub mode: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ConnectionState {
    Unknown = 0,
    Activating = 1,
    Activated = 2,
    Deactivating = 3,
    Deactivated = 4,
}

impl From<i64> for ConnectionState {
    fn from(state: i64) -> Self {
        match state {
            0 => ConnectionState::Unknown,
            1 => ConnectionState::Activating,
            2 => ConnectionState::Activated,
            3 => ConnectionState::Deactivating,
            4 => ConnectionState::Deactivated,
            _ => {
                warn!("Undefined connection state: {}", state);
                ConnectionState::Unknown
            }
        }
    }
}

pub fn get_connections(dbus_manager: &Rc<DBusNetworkManager>) -> Result<Vec<Connection>> {
    let paths = dbus_manager.list_connections()?;

    let mut connections = Vec::with_capacity(paths.len());

    for path in &paths {
        connections.push(Connection::init(dbus_manager, path)?)
    }

    connections.sort();

    Ok(connections)
}

pub fn get_active_connections(dbus_manager: &Rc<DBusNetworkManager>) -> Result<Vec<Connection>> {
    let active_paths = dbus_manager.get_active_connections()?;

    let mut connections = Vec::with_capacity(active_paths.len());

    for active_path in active_paths {
        if let Some(path) = dbus_manager.get_active_connection_path(&active_path) {
            connections.push(Connection::init(dbus_manager, &path)?)
        }
    }

    connections.sort();

    Ok(connections)
}

pub fn connect_to_access_point(
    dbus_manager: &Rc<DBusNetworkManager>,
    device_path: &str,
    access_point: &AccessPoint,
    credentials: &AccessPointCredentials,
) -> Result<(Connection, ConnectionState)> {
    let (path, _) = dbus_manager.connect_to_access_point(device_path, access_point, credentials)?;

    let connection = Connection::init(dbus_manager, &path)?;

    let state = wait(
        &connection,
        &ConnectionState::Activated,
        WIFI_TIMEOUT,
    )?;

    Ok((connection, state))
}


pub fn create_hotspot<S>(
    dbus_manager: &Rc<DBusNetworkManager>,
    device_path: &str,
    interface: &str,
    ssid: &S,
    password: Option<&str>,
    address: Option<Ipv4Addr>,
) -> Result<(Connection, ConnectionState)>
where
    S: AsSsidSlice + ?Sized,
{
    let (path, _) = dbus_manager.create_hotspot(device_path, interface, ssid, password, address)?;

    let connection = Connection::init(dbus_manager, &path)?;

    let state = wait(
        &connection,
        &ConnectionState::Activated,
        WIFI_TIMEOUT,
    )?;

    Ok((connection, state))
}

fn get_connection_active_path(
    dbus_manager: &DBusNetworkManager,
    connection_path: &str,
) -> Result<Option<String>> {
    let active_paths = dbus_manager.get_active_connections()?;

    for active_path in active_paths {
        if let Some(settings_path) = dbus_manager.get_active_connection_path(&active_path) {
            if connection_path == settings_path {
                return Ok(Some(active_path));
            }
        }
    }

    Ok(None)
}

fn wait(
    connection: &Connection,
    target_state: &ConnectionState,
    timeout: u64,
) -> Result<ConnectionState> {
    if timeout == 0 {
        return connection.get_state();
    }

    debug!("Waiting for connection state: {:?}", target_state);

    let mut total_time = 0;

    loop {
        ::std::thread::sleep(::std::time::Duration::from_secs(1));

        let state = connection.get_state()?;

        total_time += 1;

        if state == *target_state {
            debug!(
                "Connection target state reached: {:?} / {}s elapsed",
                state, total_time
            );

            return Ok(state);
        } else if total_time >= timeout {
            debug!(
                "Timeout reached in waiting for connection state ({:?}): {:?} / {}s elapsed",
                target_state, state, total_time
            );

            return Ok(state);
        }

        debug!(
            "Still waiting for connection state ({:?}): {:?} / {}s elapsed",
            target_state, state, total_time
        );
    }
}
