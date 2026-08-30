use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};

#[derive(Clone)]
pub struct ServerCrypto {
    cipher: Aes256Gcm,
}

impl ServerCrypto {
    pub fn from_base64(value: &str) -> Result<Self> {
        let bytes = STANDARD
            .decode(value)
            .context("AI_RPA_DATA_KEY must be base64")?;
        if bytes.len() != 32 {
            bail!("AI_RPA_DATA_KEY must decode to exactly 32 bytes");
        }
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&bytes).expect("validated AES-256 key"),
        })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| anyhow!("failed to encrypt remote command"))?;
        let mut envelope = nonce.to_vec();
        envelope.extend(ciphertext);
        Ok(STANDARD.encode(envelope))
    }

    pub fn decrypt(&self, envelope: &str) -> Result<String> {
        let bytes = STANDARD
            .decode(envelope)
            .context("invalid encrypted command")?;
        if bytes.len() < 12 {
            bail!("encrypted command envelope is too short");
        }
        let (nonce, ciphertext) = bytes.split_at(12);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| anyhow!("failed to decrypt remote command"))?;
        String::from_utf8(plaintext).context("remote command is not UTF-8")
    }
}
