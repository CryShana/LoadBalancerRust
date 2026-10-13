use std::fs;
use std::io::Result;
use std::net::{SocketAddr, ToSocketAddrs};

/**
    Reads `HOSTNAME:PORT` entries, one per line. Domains are resolved once, preferring an IPv4 address.
*/
pub fn load_hosts(path: &str) -> Result<Vec<SocketAddr>> {
    let mut hosts = vec![];

    for line in fs::read_to_string(path)?.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let addrs: Vec<SocketAddr> = match line.to_socket_addrs() {
            Ok(addrs) => addrs.collect(),
            Err(e) => {
                println!("[Hosts] Invalid host '{}': {}", line, e);
                continue;
            }
        };

        match addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()) {
            Some(&addr) => hosts.push(addr),
            None => println!("[Hosts] Host '{}' did not resolve to any address", line),
        }
    }

    println!("[Hosts] Registered {} valid hosts", hosts.len());
    Ok(hosts)
}
