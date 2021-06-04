use std::io::ErrorKind;
use std::io::Result;
use std::process::exit;

use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

mod balancer;
use balancer::{HostManager, LoadBalancer};
use mio::Events;
use mio::Interest;
use mio::Poll;
use mio::Token;
use mio::net::TcpListener;

use crate::balancer::RoundRobin;

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
    let mut events = Events::with_capacity(128);

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

    const SERVER_TOKEN: Token = Token(0);
    poll.registry().register(&mut listener, SERVER_TOKEN, Interest::READABLE)?;

    //listener.set_nonblocking(true).expect("Failed to put listener into non-blocking mode!");

    // START LISTENING
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
                    let connection = listener.accept()?;
                    balancer.add_client(connection.0);
                },
                _ => {}
            }
        }
    }

    Ok(())
}
