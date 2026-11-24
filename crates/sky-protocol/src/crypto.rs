use aes::Aes128;
use cfb8::{Decryptor, Encryptor};
use cipher::generic_array::GenericArray;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use num_bigint::BigInt;
use sha1::{Digest, Sha1};

/// AES-128-CFB8 stream cipher pair matching Minecraft's TCP transport encryption.
///
/// Both the key and initialization vector are initialized to the 16-byte shared secret.
pub struct MinecraftCipher {
    encryptor: Encryptor<Aes128>,
    decryptor: Decryptor<Aes128>,
}

impl MinecraftCipher {
    pub fn new(shared_secret: &[u8; 16]) -> Self {
        Self {
            encryptor: Encryptor::<Aes128>::new_from_slices(shared_secret, shared_secret)
                .expect("16-byte AES-128 key/IV is always valid"),
            decryptor: Decryptor::<Aes128>::new_from_slices(shared_secret, shared_secret)
                .expect("16-byte AES-128 key/IV is always valid"),
        }
    }

    pub fn encrypt_in_place(&mut self, data: &mut [u8]) {
        for byte in data {
            self.encryptor
                .encrypt_block_mut(GenericArray::from_mut_slice(std::slice::from_mut(byte)));
        }
    }

    pub fn decrypt_in_place(&mut self, data: &mut [u8]) {
        for byte in data {
            self.decryptor
                .decrypt_block_mut(GenericArray::from_mut_slice(std::slice::from_mut(byte)));
        }
    }
}

/// Computes Mojang's signed two's-complement SHA-1 hex digest used by `session/minecraft/join`
/// and `session/minecraft/hasJoined`.
pub fn mojang_server_hash(server_id: &str, shared_secret: &[u8], public_key_der: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(server_id.as_bytes());
    hasher.update(shared_secret);
    hasher.update(public_key_der);
    let digest = hasher.finalize();
    let bigint = BigInt::from_signed_bytes_be(&digest);
    format!("{bigint:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mojang_sha1_signed_hex_matches_wiki_vectors() {
        assert_eq!(
            mojang_server_hash("Notch", b"", b""),
            "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
        );
        assert_eq!(
            mojang_server_hash("jeb_", b"", b""),
            "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
        );
        assert_eq!(
            mojang_server_hash("simon", b"", b""),
            "88e16a1019277b15d58faf0541e11910eb756f6"
        );
    }

    #[test]
    fn aes_128_cfb8_stream_roundtrip() {
        let key = [0x42u8; 16];
        let mut alice = MinecraftCipher::new(&key);
        let mut bob = MinecraftCipher::new(&key);

        let plaintext = b"Solara-Sky encrypted chunk stream test payload!";
        let mut wire = plaintext.to_vec();
        // Encrypt in irregular chunks to prove byte-wise CFB-8 stream continuity
        alice.encrypt_in_place(&mut wire[..7]);
        alice.encrypt_in_place(&mut wire[7..19]);
        alice.encrypt_in_place(&mut wire[19..]);
        assert_ne!(&wire[..], &plaintext[..]);

        bob.decrypt_in_place(&mut wire[..3]);
        bob.decrypt_in_place(&mut wire[3..]);
        assert_eq!(&wire[..], &plaintext[..]);
    }
}
