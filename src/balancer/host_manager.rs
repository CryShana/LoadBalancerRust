use std::str;


pub struct HostManager {
    hosts: Vec<String>
}

impl HostManager {
    pub fn new(hostfile: &str) -> Self {
        // TODO: read file, if not exists - just use empty hosts, report this

        HostManager {
            hosts: vec![]
        }
    }
}