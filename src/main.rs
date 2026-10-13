use std::net::{Ipv4Addr, TcpListener};
use std::process::exit;
use std::thread;
use std::time::Duration;

mod balancer;
use balancer::{LoadBalancer, RoundRobin, load_hosts};

const HOSTS_FILE: &str = "hosts";
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

fn main() {
    // PARSE ARGUMENTS
    let Some((port, verbose)) = parse_args() else {
        eprintln!("Usage: load-balancer-rust <port> [--verbose]");
        exit(1);
    };

    // PARSE HOSTS
    let hosts = match load_hosts(HOSTS_FILE) {
        Ok(hosts) if !hosts.is_empty() => hosts,
        Ok(_) => {
            eprintln!("[Hosts] No valid hosts found in '{}'", HOSTS_FILE);
            exit(1);
        }
        Err(e) => {
            eprintln!("[Hosts] Failed to read host file '{}': {}", HOSTS_FILE, e);
            exit(1);
        }
    };

    // INITIALIZE
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).unwrap_or_else(|e| {
        eprintln!("[Listener] Failed to listen on port {}: {}", port, e);
        exit(2);
    });

    let threads = thread::available_parallelism().map_or(4, |n| n.get());
    let balancer = LoadBalancer::start(RoundRobin::new(hosts), threads, verbose).unwrap_or_else(|e| {
        eprintln!("Failed to start worker threads: {}", e);
        exit(2);
    });

    // START LISTENING
    println!("[Listener] Listening on port {} with {} worker threads", port, threads);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => balancer.add_client(stream),
            Err(e) => {
                // avoid a busy loop when accept keeps failing, e.g. when out of file descriptors
                eprintln!("[Listener] Failed to accept connection: {}", e);
                thread::sleep(ACCEPT_ERROR_BACKOFF);
            }
        }
    }
}

fn parse_args() -> Option<(u16, bool)> {
    let mut port = None;
    let mut verbose = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-v" | "--verbose" => verbose = true,
            _ if port.is_none() => port = Some(arg.parse().ok().filter(|&p| p != 0)?),
            _ => return None,
        }
    }

    Some((port?, verbose))
}
