//! The page keeps settings (satellite style) in localStorage, which is per origin — including the
//! port — so the app uses the same port every launch when it can.

/// Chosen to be unlikely to clash with anything else on a desktop.
pub const PREFERRED_PORT: u16 = 47_801;

/// `preferred` if nothing is listening on it, otherwise 0 (let the OS pick).
pub fn pick_port(preferred: u16) -> u16 {
    // Probe and release; a single-instance app makes losing the port in between very unlikely.
    match std::net::TcpListener::bind(("127.0.0.1", preferred)) {
        Ok(_) => preferred,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn uses_the_preferred_port_when_free() {
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(pick_port(free), free);
    }

    #[test]
    fn falls_back_to_any_port_when_taken() {
        let taken = TcpListener::bind("127.0.0.1:0").unwrap();
        assert_eq!(pick_port(taken.local_addr().unwrap().port()), 0);
    }
}
