use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Result;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::path::Path;
use std::str;

pub struct HostManager {
    pub hosts: Vec<SocketAddr>
}

impl HostManager {
    pub fn new(hostfile: &str) -> Self {
        if !Path::exists(Path::new(hostfile)) {
            println!(
                "[Parser] Host file '{}' does not exist. Please create it and try again.",
                hostfile
            );

            return HostManager {
                hosts: vec![]
            };
        }

        let hosts = match HostManager::parse_hosts(hostfile) {
            Ok(h) => h,
            Err(err) => {
                println!(
                    "[Parser] Failed to parse host file '{}' -> {}",
                    hostfile,
                    err.to_string()
                );
                vec![]
            }
        };

        return HostManager { hosts: hosts };
    }

    fn parse_hosts(hostfile: &str) -> Result<Vec<SocketAddr>> {
        let mut hosts: Vec<SocketAddr> = vec![];

        let file = File::open(hostfile)?;
        let bufreader = BufReader::new(file);

        for line in bufreader.lines() {
            let l = line?;
            let l = l.trim();
            if l.len() < 2 {
                continue;
            }

            // validate IP address and port - either IPv4 or IPv6 with valid port number
            // this also accepts domains and tries to resolve them, the first resolved IP is used
            let addr: Vec<SocketAddr> = match l.to_socket_addrs() {
                Ok(a) => a.collect(),
                Err(err) => {
                    println!("[Parser] Invalid host: '{}'", l);
                    continue;
                }
            };

            let mut resolved_addr: SocketAddr = addr[0];

            // if there are more than 1 IP resolved, prioritize the IPv4
            if addr.len() > 1 {
                for a in addr {
                    if a.is_ipv4() {
                        resolved_addr = a;
                        break;
                    }
                }
            }

            // push the resolved IP onto hosts list
            hosts.push(resolved_addr);
        }

        println!("[Parser] Registered {} valid hosts", hosts.len());
        Ok(hosts)
    }
}
