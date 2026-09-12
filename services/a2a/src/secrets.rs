use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretToken(String);

impl SecretToken {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn exposed(&self) -> &str {
        &self.0
    }

    pub fn into_exposed(mut self) -> String {
        std::mem::take(&mut self.0)
    }

    pub fn hash(&self) -> CredentialHash {
        CredentialHash::from_token(&self.0)
    }
}

impl From<String> for SecretToken {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl std::fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretToken([REDACTED])")
    }
}

impl Drop for SecretToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialHash(String);

impl CredentialHash {
    pub fn from_token(token: &str) -> Self {
        let digest = Sha256::digest(token.as_bytes());
        Self(hex_encode(&digest))
    }

    pub fn verify(&self, token: &str) -> bool {
        let candidate = Self::from_token(token);
        self.0.as_bytes().ct_eq(candidate.0.as_bytes()).into()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for CredentialHash {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CredentialHash([REDACTED])")
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
