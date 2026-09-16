use crate::Client;

impl Client {
    pub(super) fn flush_outgoing(&mut self) {
        use ac_net::session::Port;
        for (port, dg) in self.session.outgoing() {
            let to = if port == Port::Primary {
                self.primary
            } else {
                self.secondary
            };
            if let Some(socket) = &self.socket {
                let _ = socket.send_to(&dg, to);
            }
        }
    }
}
