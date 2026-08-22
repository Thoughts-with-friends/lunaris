//! Reading the address boxes, which LAN mode and Remote Desktop share.

/// Where a Remote Desktop session's other end is.
///
/// The port comes from [`crate::remote::Tuning::port`] and **replaces** whatever
/// the address field carries, rather than merely filling in for a missing one.
/// The address boxes are shared with LAN mode, so they usually hold that mode's
/// port; honouring it here would silently point Remote Desktop at the LAN
/// listener. One box on the pane deciding the port for this mode is easier to
/// reason about than two fields that have to agree.
pub(crate) fn parse_remote_address(text: &str, port: u16) -> Result<std::net::SocketAddr, String> {
    let ip = text
        .parse::<std::net::SocketAddr>()
        .map(|addr| addr.ip())
        .or_else(|_| text.parse::<std::net::IpAddr>())
        .map_err(|error| format!("invalid Remote Desktop address {text}: {error}"))?;
    Ok(std::net::SocketAddr::new(ip, port))
}

#[cfg(test)]
mod remote_address_tests {
    use super::parse_remote_address;

    /// The tuning's port wins, so the LAN boxes' port cannot misdirect a
    /// Remote Desktop session.
    #[test]
    pub(crate) fn the_tuned_port_replaces_whatever_the_field_holds() {
        assert_eq!(
            parse_remote_address("192.168.1.20:7064", 7065).unwrap().to_string(),
            "192.168.1.20:7065"
        );
        assert_eq!(
            parse_remote_address("192.168.1.20", 7065).unwrap().to_string(),
            "192.168.1.20:7065"
        );
        assert_eq!(parse_remote_address("0.0.0.0:1", 9000).unwrap().to_string(), "0.0.0.0:9000");
        assert!(parse_remote_address("not an address", 7065).is_err());
    }
}

pub(crate) fn parse_lan_address(
    text: &str,
    default_port: u16,
) -> Result<std::net::SocketAddr, String> {
    text.parse::<std::net::SocketAddr>()
        .or_else(|_| {
            text.parse::<std::net::IpAddr>().map(|ip| std::net::SocketAddr::new(ip, default_port))
        })
        .map_err(|error| format!("invalid LAN address {text}: {error}"))
}

#[cfg(test)]
mod lan_address_tests {
    use super::parse_lan_address;

    #[test]
    pub(crate) fn plain_ip_uses_the_default_lan_port() {
        assert_eq!(
            parse_lan_address("192.168.1.20", 7064).unwrap().to_string(),
            "192.168.1.20:7064"
        );
    }

    #[test]
    pub(crate) fn explicit_port_is_preserved() {
        assert_eq!(
            parse_lan_address("192.168.1.20:8000", 7064).unwrap().to_string(),
            "192.168.1.20:8000"
        );
    }
}
