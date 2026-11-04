use hmac::Hmac;
use sha1::Sha1;

const ITERATIONS: u32 = 4096;

pub fn derive(ssid: &str, password: &str) -> String {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2::<Hmac<Sha1>>(password.as_bytes(), ssid.as_bytes(), ITERATIONS, &mut key)
        .expect("HMAC-SHA1 accepts keys of any length");
    hex::encode(key)
}

pub fn key(ssid: &str, password: &str) -> Option<String> {
    match password {
        "" => None,
        raw if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) => {
            Some(raw.to_ascii_lowercase())
        }
        passphrase => Some(derive(ssid, passphrase)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IEEE_PSK: &str = "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e";

    #[test]
    fn ieee_80211i_test_vector() {
        assert_eq!(derive("IEEE", "password"), IEEE_PSK);
    }

    #[test]
    fn an_open_network_has_no_key() {
        assert_eq!(key("STARLINK", ""), None);
    }

    #[test]
    fn a_raw_psk_is_used_as_given() {
        assert_eq!(
            key("IEEE", &IEEE_PSK.to_ascii_uppercase()).as_deref(),
            Some(IEEE_PSK)
        );
    }

    #[test]
    fn a_passphrase_is_derived() {
        assert_eq!(key("IEEE", "password").as_deref(), Some(IEEE_PSK));
    }
}
