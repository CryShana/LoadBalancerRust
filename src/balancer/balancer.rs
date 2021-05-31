use super::TcpClient;

pub struct LoadBalancer {
    clients: [TcpClient; 16384]
}

impl LoadBalancer {
    
}