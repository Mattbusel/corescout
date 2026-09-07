//! Keeping the service running.
//!
//! # Adopt, do not duplicate
//!
//! The service may already be running: started at sign-in, or by an MCP client
//! that connected before anyone opened the window. So this connects first and
//! starts second. Two services writing to one database would be a corrupted
//! store, and the check that prevents it is this ordering.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use corescout_product_api::endpoint::Endpoint;
use corescout_product_api::Client;

/// How long to wait for a service that has just been started.
const STARTUP: Duration = Duration::from_secs(15);

/// The service, whether this process started it or found it.
#[derive(Default)]
pub struct Service {
    child: Option<Child>,
    client: Option<Client>,
}

impl Service {
    /// Nothing yet.
    pub fn new() -> Service {
        Service::default()
    }

    /// A client, starting the service if it is not already there.
    pub fn client(&mut self) -> Result<Client, String> {
        if let Some(client) = &self.client {
            if client.is_alive() {
                return Ok(client.clone());
            }
            self.client = None;
        }
        self.ensure_running()?;
        self.client
            .clone()
            .ok_or_else(|| "CoreScout's service is not answering".to_string())
    }

    /// Make sure a service is running, adopting one that already is.
    pub fn ensure_running(&mut self) -> Result<(), String> {
        if let Ok(client) = Client::connect() {
            if client.is_alive() {
                self.client = Some(client);
                return Ok(());
            }
        }
        self.child = Some(spawn()?);
        let deadline = Instant::now() + STARTUP;
        while Instant::now() < deadline {
            if let Ok(client) = Client::connect() {
                if client.is_alive() {
                    self.client = Some(client);
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(120));
        }
        Err("CoreScout's service did not start. Try quitting and opening it again.".into())
    }

    /// Stop the service, if this process started it.
    ///
    /// A service that was already running when the window opened is left
    /// alone: something else is depending on it, and stopping it here would
    /// disconnect an agent mid-session.
    pub fn stop(&mut self) {
        self.client = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            Endpoint::withdraw();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start the service that ships alongside this application.
fn spawn() -> Result<Child, String> {
    let exe = std::env::current_exe().map_err(|error| format!("cannot find myself: {error}"))?;
    let directory = exe
        .parent()
        .ok_or_else(|| "cannot find my own folder".to_string())?;
    let service = directory.join(if cfg!(windows) {
        "corescout-service.exe"
    } else {
        "corescout-service"
    });
    if !service.exists() {
        return Err(format!(
            "{} is missing. Reinstalling CoreScout will restore it.",
            service.display()
        ));
    }
    let mut command = Command::new(&service);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    no_window(&mut command);
    command
        .spawn()
        .map_err(|error| format!("could not start CoreScout's service: {error}"))
}

/// Start it without a console window appearing.
#[cfg(windows)]
fn no_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_command: &mut Command) {}
