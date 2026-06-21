//! WPA-PSK derivation so the plaintext WiFi passphrase is never written to the
//! card. `PSK = PBKDF2(HMAC-SHA1, passphrase, ssid, 4096, 32 bytes)`, hex-encoded
//! — identical to `wpa_passphrase`.

use hmac::Hmac;
use sha1::Sha1;

/// Number of PBKDF2 iterations mandated by IEEE 802.11i for WPA-PSK.
const ITERATIONS: u32 = 4096;

/// Derive the 64-hex-char WPA-PSK from an SSID + passphrase.
pub fn derive(ssid: &str, password: &str) -> String {
    let mut key = [0u8; 32];
    // pbkdf2 with HMAC-SHA1: passphrase is the password, SSID is the salt.
    pbkdf2::pbkdf2::<Hmac<Sha1>>(password.as_bytes(), ssid.as_bytes(), ITERATIONS, &mut key)
        .expect("HMAC-SHA1 accepts keys of any length");
    hex::encode(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ieee_80211i_test_vector() {
        // IEEE 802.11i / RFC-known WPA-PSK test vector.
        assert_eq!(
            derive("IEEE", "password"),
            "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e"
        );
    }
}
