use serde::{Deserialize, Serialize};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use ed25519_dalek::{Signer, Verifier, SigningKey, VerifyingKey, Signature};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519SecretKey};
use sha2::{Sha256, Digest};
use rand::RngCore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keypair {
    pub encrypt_pk: Vec<u8>,
    pub encrypt_sk: Vec<u8>,
    pub sign_pk: Vec<u8>,
    pub sign_sk: Vec<u8>,
}

/// Initialize crypto (no-op for pure Rust, kept for compatibility)
pub fn init() -> Result<(), String> {
    Ok(())
}

/// Generate a new keypair for encryption and signing
pub fn generate_keypair() -> Keypair {
    // Generate X25519 keypair for encryption
    let encrypt_sk = X25519SecretKey::random_from_rng(OsRng);
    let encrypt_pk = X25519PublicKey::from(&encrypt_sk);

    // Generate Ed25519 keypair for signing
    let sign_sk = SigningKey::generate(&mut OsRng);
    let sign_pk = sign_sk.verifying_key();

    Keypair {
        encrypt_pk: encrypt_pk.as_bytes().to_vec(),
        encrypt_sk: encrypt_sk.to_bytes().to_vec(),
        sign_pk: sign_pk.to_bytes().to_vec(),
        sign_sk: sign_sk.to_bytes().to_vec(),
    }
}

/// Encrypt a message using X25519 + XChaCha20-Poly1305
/// Returns nonce prepended to ciphertext
pub fn encrypt_message(
    plaintext: &[u8],
    recipient_pk: &[u8],
    sender_sk: &[u8],
) -> Result<Vec<u8>, String> {
    if recipient_pk.len() != 32 {
        return Err("Invalid recipient public key length".to_string());
    }
    if sender_sk.len() != 32 {
        return Err("Invalid sender secret key length".to_string());
    }

    // Perform X25519 key exchange
    let recipient_pk_bytes: [u8; 32] = recipient_pk.try_into().unwrap();
    let sender_sk_bytes: [u8; 32] = sender_sk.try_into().unwrap();

    let recipient_pk = X25519PublicKey::from(recipient_pk_bytes);
    let sender_sk = X25519SecretKey::from(sender_sk_bytes);
    let shared_secret = sender_sk.diffie_hellman(&recipient_pk);

    // Generate random nonce (24 bytes for XChaCha20)
    let mut nonce_bytes = [0u8; 24];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);

    // Encrypt using shared secret
    let cipher = XChaCha20Poly1305::new(shared_secret.as_bytes().into());
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| "Encryption failed".to_string())?;

    // Prepend nonce to ciphertext
    let mut result = nonce_bytes.to_vec();
    result.extend(ciphertext);

    Ok(result)
}

/// Decrypt a message using X25519 + XChaCha20-Poly1305
/// Expects nonce prepended to ciphertext
pub fn decrypt_message(
    ciphertext: &[u8],
    sender_pk: &[u8],
    recipient_sk: &[u8],
) -> Result<Vec<u8>, String> {
    const NONCE_SIZE: usize = 24;

    if ciphertext.len() < NONCE_SIZE {
        return Err("Invalid ciphertext: too short".to_string());
    }
    if sender_pk.len() != 32 {
        return Err("Invalid sender public key length".to_string());
    }
    if recipient_sk.len() != 32 {
        return Err("Invalid recipient secret key length".to_string());
    }

    // Perform X25519 key exchange
    let sender_pk_bytes: [u8; 32] = sender_pk.try_into().unwrap();
    let recipient_sk_bytes: [u8; 32] = recipient_sk.try_into().unwrap();

    let sender_pk = X25519PublicKey::from(sender_pk_bytes);
    let recipient_sk = X25519SecretKey::from(recipient_sk_bytes);
    let shared_secret = recipient_sk.diffie_hellman(&sender_pk);

    // Extract nonce and ciphertext
    let nonce = XNonce::from_slice(&ciphertext[..NONCE_SIZE]);
    let sealed = &ciphertext[NONCE_SIZE..];

    // Decrypt using shared secret
    let cipher = XChaCha20Poly1305::new(shared_secret.as_bytes().into());
    cipher
        .decrypt(nonce, sealed)
        .map_err(|_| "Decryption failed".to_string())
}

/// Sign a message using Ed25519
pub fn sign_message(message: &[u8], sign_sk: &[u8]) -> Result<Vec<u8>, String> {
    if sign_sk.len() != 32 {
        return Err("Invalid signing secret key length".to_string());
    }

    let sk_bytes: [u8; 32] = sign_sk.try_into().unwrap();
    let signing_key = SigningKey::from_bytes(&sk_bytes);
    let signature = signing_key.sign(message);

    // Return signature + message (compatible with sodiumoxide format)
    let mut result = signature.to_bytes().to_vec();
    result.extend(message);
    Ok(result)
}

/// Verify a signature using Ed25519
/// Returns the original message if signature is valid
pub fn verify_signature(signed_message: &[u8], sign_pk: &[u8]) -> Result<Vec<u8>, String> {
    const SIGNATURE_SIZE: usize = 64;

    if signed_message.len() < SIGNATURE_SIZE {
        return Err("Invalid signed message: too short".to_string());
    }
    if sign_pk.len() != 32 {
        return Err("Invalid signing public key length".to_string());
    }

    let pk_bytes: [u8; 32] = sign_pk.try_into().unwrap();
    let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
        .map_err(|_| "Invalid public key".to_string())?;

    // Extract signature and message
    let signature_bytes: [u8; SIGNATURE_SIZE] = signed_message[..SIGNATURE_SIZE]
        .try_into()
        .unwrap();
    let signature = Signature::from_bytes(&signature_bytes);
    let message = &signed_message[SIGNATURE_SIZE..];

    // Verify signature
    verifying_key
        .verify(message, &signature)
        .map_err(|_| "Signature verification failed".to_string())?;

    Ok(message.to_vec())
}

/// Convert bytes to hex string
pub fn to_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

/// Convert hex string to bytes
pub fn from_hex(hex_str: &str) -> Result<Vec<u8>, String> {
    hex::decode(hex_str).map_err(|e| format!("Invalid hex: {}", e))
}

/// Generate deterministic queue ID from two public keys
/// Both users will get the same queue ID regardless of order
pub fn generate_conversation_queue_id(pk1_hex: &str, pk2_hex: &str) -> Result<String, String> {
    // Sort keys lexicographically to ensure same order for both users
    let (min_pk, max_pk) = if pk1_hex < pk2_hex {
        (pk1_hex, pk2_hex)
    } else {
        (pk2_hex, pk1_hex)
    };

    // Concatenate sorted keys
    let combined = format!("{}{}", min_pk, max_pk);

    // Hash the combination using SHA256
    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    let hash = hasher.finalize();

    // Use first 16 bytes as queue_id
    let queue_id = hex::encode(&hash[..16]);

    Ok(queue_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        init().unwrap();

        let alice = generate_keypair();
        let bob = generate_keypair();

        let plaintext = b"Hello, Bob!";

        // Alice encrypts for Bob
        let ciphertext = encrypt_message(
            plaintext,
            &bob.encrypt_pk,
            &alice.encrypt_sk,
        ).unwrap();

        // Bob decrypts from Alice
        let decrypted = decrypt_message(
            &ciphertext,
            &alice.encrypt_pk,
            &bob.encrypt_sk,
        ).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_sign_verify_roundtrip() {
        init().unwrap();

        let alice = generate_keypair();
        let message = b"I am Alice";

        // Alice signs the message
        let signed = sign_message(message, &alice.sign_sk).unwrap();

        // Anyone can verify Alice's signature
        let verified = verify_signature(&signed, &alice.sign_pk).unwrap();

        assert_eq!(message.to_vec(), verified);
    }

    #[test]
    fn test_hex_conversion() {
        let data = b"test data";
        let hex = to_hex(data);
        let decoded = from_hex(&hex).unwrap();
        assert_eq!(data.to_vec(), decoded);
    }

    #[test]
    fn test_deterministic_queue_id() {
        init().unwrap();

        let alice = generate_keypair();
        let bob = generate_keypair();

        let alice_pk_hex = to_hex(&alice.encrypt_pk);
        let bob_pk_hex = to_hex(&bob.encrypt_pk);

        // Generate queue_id from Alice's perspective
        let queue_id_1 = generate_conversation_queue_id(&alice_pk_hex, &bob_pk_hex).unwrap();

        // Generate queue_id from Bob's perspective (reversed order)
        let queue_id_2 = generate_conversation_queue_id(&bob_pk_hex, &alice_pk_hex).unwrap();

        // Both should produce the same queue_id
        assert_eq!(queue_id_1, queue_id_2);
        println!("Deterministic queue_id: {}", queue_id_1);
    }
}
