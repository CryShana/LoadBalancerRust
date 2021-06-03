use std::io::Result;
use std::net::TcpListener;
use std::process::exit;

use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

mod balancer;
use balancer::{HostManager, LoadBalancer};

use crate::balancer::RoundRobin;

const SLEEP_TIME: Duration = Duration::from_millis(5);

fn main() -> Result<()> {
    // PARSE HOSTS
    let host_manager = HostManager::new("hosts");
    if host_manager.hosts.len() == 0 {
        return Ok(());
    }

    // PREPARE THE LOAD BALANCER
    let debug_mode = true;
    let number_of_threads = 4;
    let round_robin = RoundRobin::new(host_manager);
    let mut balancer = LoadBalancer::new(round_robin, number_of_threads, debug_mode);

    // PREPARE THE CTRL+C HANDLER FOR GRACEFUL STOP
    let should_cancel = Arc::new(Mutex::new(false));
    let cancel = Arc::clone(&should_cancel);
    ctrlc::set_handler(move || {
        *cancel.lock().unwrap() = true;
    })
    .expect("Failed to set Ctrl+C handler!");

    // PARSE THE PROVIDED LISTENING PORT
    let listening_port = match std::env::args().nth(1) {
        Some(arg) => arg,
        None => {
            println!("Please specify local port to listen on!");
            exit(1);
        }
    };

    // BIND TO LISTENING PORT
    let addr = format!("0.0.0.0:{}", listening_port);
    let listener: TcpListener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(err) => {
            println!("Failed to bind to port {} -> {}", listening_port, err.to_string());
            exit(2);
        }
    };
    listener.set_nonblocking(true).expect("Failed to put listener into non-blocking mode!");

    // START LISTENING
    println!("[Listener] Started listening on port {}", listening_port);
    for stream in listener.incoming() {
        match stream {
            Ok(str) => {
                balancer.add_client(str);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(err) => {
                println!("Failed to accept connection! {}", err.to_string());
            }
        }

        if *should_cancel.lock().unwrap() == true {
            println!("[Listener] Listening stopped");
            balancer.stop();

            // sleep a bit to allow all threads to exit gracefully
            thread::sleep(SLEEP_TIME);

            break;
        }

        thread::sleep(SLEEP_TIME);
    }

    Ok(())
}
