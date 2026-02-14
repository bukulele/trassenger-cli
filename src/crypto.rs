use sodiumoxide::crypto::{box_, sign};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keypair {
    pub encrypt_pk: Vec<u8>,
    pub encrypt_sk: Vec<u8>,
    pub sign_pk: Vec<u8>,
    pub sign_sk: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublicKeyInfo {
    pub encrypt_pk_hex: String,
    pub sign_pk_hex: String,
}

/// Initialize sodiumoxide (must be called before any crypto operations)
pub fn init() -> Result<(), String> {
    sodiumoxide::init().map_err(|_| "Failed to initialize crypto library".to_string())
}

/// Generate a new keypair for encryption and signing
pub fn generate_keypair() -> Keypair {
    let (encrypt_pk, encrypt_sk) = box_::gen_keypair();
    let (sign_pk, sign_sk) = sign::gen_keypair();

    Keypair {
        encrypt_pk: encrypt_pk.0.to_vec(),
        encrypt_sk: encrypt_sk.0.to_vec(),
        sign_pk: sign_pk.0.to_vec(),
        sign_sk: sign_sk.0.to_vec(),
    }
}

/// Encrypt a message using X25519 + XChaCha20-Poly1305
/// Returns nonce prepended to ciphertext
pub fn encrypt_message(
    plaintext: &[u8],
    recipient_pk: &[u8],
    sender_sk: &[u8],
) -> Result<Vec<u8>, String> {
    let recipient_pk = box_::PublicKey::from_slice(recipient_pk)
        .ok_or("Invalid recipient public key")?;
    let sender_sk = box_::SecretKey::from_slice(sender_sk)
        .ok_or("Invalid sender secret key")?;

    let nonce = box_::gen_nonce();
    let ciphertext = box_::seal(plaintext, &nonce, &recipient_pk, &sender_sk);

    // Prepend nonce to ciphertext
    let mut result = nonce.0.to_vec();
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
    if ciphertext.len() < box_::NONCEBYTES {
        return Err("Invalid ciphertext: too short".to_string());
    }

    let sender_pk = box_::PublicKey::from_slice(sender_pk)
        .ok_or("Invalid sender public key")?;
    let recipient_sk = box_::SecretKey::from_slice(recipient_sk)
        .ok_or("Invalid recipient secret key")?;

    // Extract nonce from first NONCEBYTES
    let nonce = box_::Nonce::from_slice(&ciphertext[..box_::NONCEBYTES])
        .ok_or("Invalid nonce")?;
    let sealed = &ciphertext[box_::NONCEBYTES..];

    box_::open(sealed, &nonce, &sender_pk, &recipient_sk)
        .map_err(|_| "Decryption failed".to_string())
}

/// Sign a message using Ed25519
pub fn sign_message(message: &[u8], sign_sk: &[u8]) -> Result<Vec<u8>, String> {
    let sign_sk = sign::SecretKey::from_slice(sign_sk)
        .ok_or("Invalid signing secret key")?;

    Ok(sign::sign(message, &sign_sk))
}

/// Verify a signature using Ed25519
/// Returns the original message if signature is valid
pub fn verify_signature(signed_message: &[u8], sign_pk: &[u8]) -> Result<Vec<u8>, String> {
    let sign_pk = sign::PublicKey::from_slice(sign_pk)
        .ok_or("Invalid signing public key")?;

    sign::verify(signed_message, &sign_pk)
        .map_err(|_| "Signature verification failed".to_string())
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
    use sodiumoxide::crypto::hash::sha256;

    // Sort keys lexicographically to ensure same order for both users
    let (min_pk, max_pk) = if pk1_hex < pk2_hex {
        (pk1_hex, pk2_hex)
    } else {
        (pk2_hex, pk1_hex)
    };

    // Concatenate sorted keys
    let combined = format!("{}{}", min_pk, max_pk);

    // Hash the combination
    let hash = sha256::hash(combined.as_bytes());

    // Convert to hex and use first 32 chars as queue_id
    let queue_id = hex::encode(&hash.0[..16]);

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
