use std::fs;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Result;
use std::path::Path;
use std::str;

pub struct HostManager {
    pub hosts: Vec<String>,
}

impl HostManager {
    pub fn new(hostfile: &str) -> Self {
        if !Path::exists(Path::new(hostfile)) {
            println!(
                "Host file '{}' does not exist. Please create it and try again.",
                hostfile
            );

            return HostManager { hosts: vec![] }
        }

        let hosts = match HostManager::parse_hosts(hostfile) {
            Ok(h) => h,
            Err(err) => {
                println!(
                    "Failed to parse host file '{}' -> {}",
                    hostfile,
                    err.to_string()
                );
                vec![]
            }
        };

        return HostManager { hosts: hosts }
    }

    fn parse_hosts(hostfile: &str) -> Result<Vec<String>> {
        let mut hosts: Vec<String> = vec![];

        let file = File::open(hostfile)?;
        let bufreader = BufReader::new(file);

        // TODO: parse CSV file
        for line in bufreader.lines() {
            let l = line?;
            if l.len() < 2 {
                continue;
            }

            // TODO: validate IP address and port - either IPv4 or IPv6 with valid port number
            println!("Line is {}", l);

            hosts.push(l);
        }

        Ok(hosts)
    }
}
