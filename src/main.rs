use std::io::ErrorKind;
use std::io::Result;
use std::process::exit;

use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

mod balancer;
use balancer::{HostManager, LoadBalancer};
use mio::net::TcpListener;
use mio::Events;
use mio::Interest;
use mio::Poll;
use mio::Token;

use crate::balancer::RoundRobin;

const SERVER_TOKEN: Token = Token(0);
const CLIENT_TOKEN: Token = Token(1);

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

    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(1024);

    let addr = match format!("0.0.0.0:{}", listening_port).parse() {
        Ok(a) => a,
        Err(_) => {
            println!("Invalid listening port provided!");
            exit(1)
        }
    };

    let mut listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(err) => {
            println!("Failed to bind to port {} -> {}", listening_port, err.to_string());
            exit(2);
        }
    };

    poll.registry().register(&mut listener, SERVER_TOKEN, Interest::READABLE)?;

    // START LISTENING
    println!("[Listener] Started listening on port {}", listening_port);
    loop {
        // POLL FOR EVENTS HERE
        match poll.poll(&mut events, None) {
            Ok(_) => {}
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {
                *should_cancel.lock().unwrap() = true;
                balancer.stop();

                println!("[Listener] Listening stopped");

                // sleep a bit to allow all threads to exit gracefully
                thread::sleep(Duration::from_millis(4));

                break;
            }
            Err(e) => {
                println!("Failed to poll for listener events! {}", e.to_string());
                break;
            }
        };

        for event in events.iter() {
            match event.token() {
                SERVER_TOKEN => {
                    // listener accepted a new client
                    let mut connection = listener.accept()?;

                    poll.registry()
                        .register(&mut connection.0, CLIENT_TOKEN, Interest::READABLE | Interest::WRITABLE)?;

                    balancer.add_client(connection.0);
                }
                CLIENT_TOKEN => {
                    // notify balancer of a change, wake it up

                    if event.is_writable() {
                        // We can (likely) write to the socket without blocking.
                    }

                    if event.is_readable() {
                        // We can (likely) read from the socket without blocking.
                    }

                    balancer.wake_up();
                }
                _ => {}
            }
        }
    }

    Ok(())
}
