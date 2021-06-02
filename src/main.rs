use std::io::{prelude::*, Result};
use std::net::TcpListener;
use std::process::exit;

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{self, Duration};
use std::{env, thread};

mod balancer;
use balancer::{LoadBalancer, HostManager};

use crate::balancer::RoundRobin;

const SLEEP_TIME: Duration = Duration::from_millis(5);

fn main() -> Result<()> {
    // - file that contains list of hosts in format [IP]:[Port]
    // - load balancing algorithm is also given as an argument - default is round robin
    // - use threadpool to handle clients

    // algorithm
    // - handle clients by IP / or by connection / or by time period?
    // - clients are assigned hosts as they connect and stay with those hosts (assigned host is determined by the method used)

    // IDEA: save new clients to a pre-allocated array OR list --- multiple threads are then spawned and handle clients in this array (because it's non-blocking, can jsut go through many of them)
    // - Need a way to remove inactive clients --> timeouts on no receive or sending data?

    let host_manager = HostManager::new("hosts");
    if host_manager.hosts.len() == 0 {
        return Ok(());
    }

    let round_robin = RoundRobin::new(host_manager);
    let mut balancer = LoadBalancer::new(&round_robin, 4);

    let should_cancel = Arc::new(Mutex::new(false));
    let cancel = Arc::clone(&should_cancel);
    ctrlc::set_handler(move || {
        *cancel.lock().unwrap() = true;
    })
    .expect("Failed to set Ctrl+C handler!");

    // get endpoint
    let listening_port = match std::env::args().nth(1) {
        Some(arg) => arg,
        None => {
            println!("Please specify local port to listen on!");
            exit(1);
        }
    };

    let addr = format!("0.0.0.0:{}", listening_port);

    let listener: TcpListener = TcpListener::bind(addr).expect("Failed to bind to port!");
    listener
        .set_nonblocking(true)
        .expect("Failed to put listener into non-blocking mode!");

    // accept connections and process them serially
    println!("Started listening on port {}", listening_port);
    for stream in listener.incoming() {
        match stream {
            Ok(str) => {
                balancer.add_client(str);
            }
            // because we are not blocking (to exit gracefully), we need to ignore non-blocking errors
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            // handle actual errors here
            Err(err) => {
                println!("Failed to accept connection! {}", err.to_string());
            }
        }

        if *should_cancel.lock().unwrap() == true {
            println!("Listening stopped");
            balancer.stop();

            // sleep a bit to allow all threads to exit gracefully
            thread::sleep(SLEEP_TIME);

            break;
        }

        thread::sleep(SLEEP_TIME);
    }

    Ok(())
}
