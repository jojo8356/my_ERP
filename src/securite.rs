//! Sécurité locale sans dépendance cryptographique externe (CDC §10).
//!
//! - **SEC-01** mots de passe hachés (PBKDF2-HMAC-SHA256, sel aléatoire,
//!   120 000 itérations) : aucun mot de passe en clair n'est stocké, ni dans
//!   les journaux (le CLI journalise l'action, jamais le secret).
//! - **SEC-02** 2FA TOTP (RFC 6238) obligatoire pour les comptes admin, plus
//!   des codes de récupération à usage unique.
//! - **SEC-04** rôles simples Admin / Utilisateur.
//! - **SEC-05** empreintes chaînées du journal d'audit (SHA-256).
//!
//! ⚠️ Limites assumées et documentées (ADR-007) : PBKDF2 plutôt qu'Argon2id
//! (pas de dépendance externe dans cet environnement), aléa dérivé de
//! l'horloge et du processus (pas de CSPRNG du système), TOTP en SHA-1
//! (compatibilité maximale avec les applications d'authentification).

use crate::orm;

// ------------------------------------------------------------ Rôles

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Utilisateur,
}

impl Role {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Utilisateur => "utilisateur",
        }
    }
    pub fn libelle(&self) -> &'static str {
        match self {
            Self::Admin => "administrateur",
            Self::Utilisateur => "utilisateur",
        }
    }
    pub fn depuis_code(code: &str) -> Self {
        match code.trim().to_lowercase().as_str() {
            "admin" | "administrateur" => Self::Admin,
            _ => Self::Utilisateur,
        }
    }
    pub fn est_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }
}

// ------------------------------------------------------------ Encodages

pub fn hex(octets: &[u8]) -> String {
    let mut sortie = String::with_capacity(octets.len() * 2);
    for o in octets {
        sortie.push_str(&format!("{o:02x}"));
    }
    sortie
}

pub fn depuis_hex(texte: &str) -> Vec<u8> {
    let octets: Vec<u8> = texte.as_bytes().to_vec();
    let mut sortie = Vec::with_capacity(octets.len() / 2);
    let mut i = 0;
    while i + 1 < octets.len() {
        let (haut, bas) = (chiffre_hex(octets[i]), chiffre_hex(octets[i + 1]));
        match (haut, bas) {
            (Some(h), Some(b)) => sortie.push(h * 16 + b),
            _ => break,
        }
        i += 2;
    }
    sortie
}

fn chiffre_hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ------------------------------------------------------------ Aléa local

/// Aléa local : horloge haute résolution + identité du processus + compteur
/// atomique, condensés par SHA-256. Suffisant pour un sel et un secret TOTP
/// en mono-utilisateur ; ce n'est **pas** un générateur cryptographique
/// système (limite documentée, ADR-007).
pub fn alea_octets(nombre: usize) -> Vec<u8> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COMPTEUR: AtomicU64 = AtomicU64::new(0);
    let compteur = COMPTEUR.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let graine = format!("{nanos}-{pid}-{compteur}");
    let mut sortie = Vec::new();
    let mut bloc = 0u32;
    while sortie.len() < nombre {
        let empreinte = orm::sha256(format!("{graine}-{bloc}").as_bytes());
        sortie.extend_from_slice(&depuis_hex(&empreinte));
        bloc += 1;
    }
    sortie.truncate(nombre);
    sortie
}

// ------------------------------------------------------------ SHA-1 (TOTP)

/// SHA-1 minimal, uniquement pour HMAC-SHA1/TOTP (compatibilité applications).
pub fn sha1(donnees: &[u8]) -> Vec<u8> {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = donnees.to_vec();
    let bits = (message.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());
    for bloc in message.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                bloc[i * 4],
                bloc[i * 4 + 1],
                bloc[i * 4 + 2],
                bloc[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut sortie = Vec::with_capacity(20);
    for v in h {
        sortie.extend_from_slice(&v.to_be_bytes());
    }
    sortie
}

fn sha256_octets(donnees: &[u8]) -> Vec<u8> {
    depuis_hex(&orm::sha256(donnees))
}

fn hmac(cle: &[u8], message: &[u8], hacher: fn(&[u8]) -> Vec<u8>, taille_bloc: usize) -> Vec<u8> {
    let mut cle = cle.to_vec();
    if cle.len() > taille_bloc {
        cle = hacher(&cle);
    }
    cle.resize(taille_bloc, 0);
    let mut interieur = Vec::with_capacity(taille_bloc + message.len());
    for octet in &cle {
        interieur.push(octet ^ 0x36);
    }
    interieur.extend_from_slice(message);
    let h1 = hacher(&interieur);
    let mut exterieur = Vec::with_capacity(taille_bloc + h1.len());
    for octet in &cle {
        exterieur.push(octet ^ 0x5c);
    }
    exterieur.extend_from_slice(&h1);
    hacher(&exterieur)
}

pub fn hmac_sha256(cle: &[u8], message: &[u8]) -> Vec<u8> {
    hmac(cle, message, sha256_octets, 64)
}

pub fn hmac_sha1(cle: &[u8], message: &[u8]) -> Vec<u8> {
    hmac(cle, message, sha1, 64)
}

// ------------------------------------------------------------ Mots de passe (SEC-01)

const ITERATIONS_PBKDF2: u32 = 120_000;

/// PBKDF2-HMAC-SHA256, un seul bloc de sortie (32 octets suffisent pour un mot de passe).
pub fn pbkdf2_sha256(mot_de_passe: &str, sel: &[u8], iterations: u32) -> Vec<u8> {
    let mut message = sel.to_vec();
    message.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(mot_de_passe.as_bytes(), &message);
    let mut sortie = u.clone();
    for _ in 1..iterations.max(1) {
        u = hmac_sha256(mot_de_passe.as_bytes(), &u);
        for (i, octet) in u.iter().enumerate() {
            sortie[i] ^= octet;
        }
    }
    sortie
}

/// Empreinte stockée : `pbkdf2-sha256$itérations$sel_hex$hash_hex`.
pub fn hacher_mot_de_passe(mot_de_passe: &str) -> String {
    let sel = alea_octets(16);
    let hash = pbkdf2_sha256(mot_de_passe, &sel, ITERATIONS_PBKDF2);
    format!(
        "pbkdf2-sha256${ITERATIONS_PBKDF2}${}${}",
        hex(&sel),
        hex(&hash)
    )
}

/// Comparaison à temps constant (aucune sortie anticipée sur le contenu).
pub fn egalite_constante(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        difference |= x ^ y;
    }
    difference == 0
}

pub fn verifier_mot_de_passe(mot_de_passe: &str, empreinte: &str) -> bool {
    let morceaux: Vec<&str> = empreinte.split('$').collect();
    if morceaux.len() != 4 || morceaux[0] != "pbkdf2-sha256" {
        return false;
    }
    let iterations: u32 = match morceaux[1].parse() {
        Ok(i) => i,
        Err(_) => return false,
    };
    let sel = depuis_hex(morceaux[2]);
    let attendu = depuis_hex(morceaux[3]);
    let calcule = pbkdf2_sha256(mot_de_passe, &sel, iterations);
    egalite_constante(&calcule, &attendu)
}

/// Politique de robustesse (SEC-01) : 12 caractères, trois familles minimum.
pub fn politique_mot_de_passe(mot_de_passe: &str) -> Result<(), String> {
    if mot_de_passe.chars().count() < 12 {
        return Err("mot de passe trop court : 12 caractères minimum".into());
    }
    let mut minuscule = false;
    let mut majuscule = false;
    let mut chiffre = false;
    let mut autre = false;
    for c in mot_de_passe.chars() {
        if c.is_ascii_lowercase() {
            minuscule = true;
        } else if c.is_ascii_uppercase() {
            majuscule = true;
        } else if c.is_ascii_digit() {
            chiffre = true;
        } else {
            autre = true;
        }
    }
    let familles = [minuscule, majuscule, chiffre, autre]
        .iter()
        .filter(|v| **v)
        .count();
    if familles < 3 {
        return Err(
            "mot de passe trop simple : minuscules, majuscules, chiffres et symboles (3 familles sur 4)"
                .into(),
        );
    }
    Ok(())
}

// ------------------------------------------------------------ TOTP (SEC-02)

const ALPHABET_BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32(octets: &[u8]) -> String {
    let mut sortie = String::new();
    let mut tampon: u32 = 0;
    let mut bits = 0;
    for octet in octets {
        tampon = (tampon << 8) | u32::from(*octet);
        bits += 8;
        while bits >= 5 {
            let index = ((tampon >> (bits - 5)) & 0x1f) as usize;
            sortie.push(ALPHABET_BASE32[index] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        let index = ((tampon << (5 - bits)) & 0x1f) as usize;
        sortie.push(ALPHABET_BASE32[index] as char);
    }
    sortie
}

pub fn base32_decode(texte: &str) -> Vec<u8> {
    let mut sortie = Vec::new();
    let mut tampon: u32 = 0;
    let mut bits = 0;
    for c in texte.chars() {
        if c == '=' || c.is_whitespace() {
            continue;
        }
        let valeur = match ALPHABET_BASE32
            .iter()
            .position(|a| *a == c.to_ascii_uppercase() as u8)
        {
            Some(v) => v as u32,
            None => continue,
        };
        tampon = (tampon << 5) | valeur;
        bits += 5;
        if bits >= 8 {
            sortie.push(((tampon >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    sortie
}

/// Nouveau secret TOTP : 20 octets aléatoires en base32 (160 bits).
pub fn nouveau_secret_totp() -> String {
    base32(&alea_octets(20))
}

/// Code TOTP à 6 chiffres pour l'instant donné (secondes Unix).
pub fn totp(secret_base32: &str, secondes: u64) -> String {
    let cle = base32_decode(secret_base32);
    let compteur = secondes / 30;
    let code = hmac_sha1(&cle, &compteur.to_be_bytes());
    let decalage = (code[19] & 0x0f) as usize;
    let binaire = (u32::from(code[decalage] & 0x7f) << 24)
        | (u32::from(code[decalage + 1]) << 16)
        | (u32::from(code[decalage + 2]) << 8)
        | u32::from(code[decalage + 3]);
    format!("{:06}", binaire % 1_000_000)
}

pub fn maintenant_secondes() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Vérifie un code TOTP avec une tolérance de ±1 période (30 s).
pub fn verifier_totp(secret_base32: &str, code: &str) -> bool {
    let code = code.trim();
    if secret_base32.trim().is_empty() || code.len() != 6 {
        return false;
    }
    let maintenant = maintenant_secondes();
    for delta in [-30i64, 0, 30] {
        let instant = (maintenant as i64 + delta).max(0) as u64;
        if egalite_constante(totp(secret_base32, instant).as_bytes(), code.as_bytes()) {
            return true;
        }
    }
    false
}

/// Lien `otpauth://` à afficher (ou à saisir dans une application d'authentification).
pub fn uri_otpauth(compte: &str, secret: &str, emetteur: &str) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&digits=6&period=30",
        encoder_uri(emetteur),
        encoder_uri(compte),
        secret,
        encoder_uri(emetteur)
    )
}

fn encoder_uri(s: &str) -> String {
    let mut sortie = String::new();
    for octet in s.bytes() {
        match octet {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                sortie.push(octet as char)
            }
            b' ' => sortie.push_str("%20"),
            autre => sortie.push_str(&format!("%{autre:02X}")),
        }
    }
    sortie
}

/// Codes de récupération à usage unique (affichés une seule fois).
pub fn nouveaux_codes_recuperation(nombre: usize) -> Vec<String> {
    let mut codes = Vec::new();
    for _ in 0..nombre {
        let brut = hex(&alea_octets(4));
        codes.push(format!("{}-{}", &brut[..4], &brut[4..8]));
    }
    codes
}

/// Empreinte courte d'un code de récupération (stockée, jamais le code).
pub fn empreinte_code(code: &str) -> String {
    orm::empreinte(&format!("recuperation:{}", code.trim().to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hachage_de_mot_de_passe_aller_retour() {
        let empreinte = hacher_mot_de_passe("Correct-Horse-2026!");
        assert!(empreinte.starts_with("pbkdf2-sha256$"));
        assert!(verifier_mot_de_passe("Correct-Horse-2026!", &empreinte));
        assert!(!verifier_mot_de_passe("mauvais", &empreinte));
        // Deux hachages du même mot de passe diffèrent (sel aléatoire).
        assert_ne!(empreinte, hacher_mot_de_passe("Correct-Horse-2026!"));
        assert!(!verifier_mot_de_passe("x", "empreinte-corrompue"));
    }

    #[test]
    fn politique_de_robustesse() {
        assert!(politique_mot_de_passe("Correct-Horse-2026!").is_ok());
        assert!(politique_mot_de_passe("court").is_err());
        assert!(politique_mot_de_passe("toutminusculeuniquement").is_err());
    }

    #[test]
    fn sha1_vecteurs_officiels() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
    }

    #[test]
    fn hmac_sha256_vecteur_rfc4231() {
        // Cas 2 de la RFC 4231 : clé « Jefe », données « what do ya want for nothing? »
        let attendu = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            attendu
        );
    }

    #[test]
    fn totp_vecteur_rfc6238() {
        // Graine RFC 6238 (« 12345678901234567890 » en base32) à t = 59 s.
        let secret = base32(b"12345678901234567890");
        assert_eq!(totp(&secret, 59), "287082");
        assert_eq!(totp(&secret, 1111111109), "081804");
        // Le contrôle « maintenant » doit accepter le code courant.
        let courant = totp(&secret, maintenant_secondes());
        assert!(verifier_totp(&secret, &courant));
        assert!(!verifier_totp(&secret, "000000"));
    }

    #[test]
    fn base32_aller_retour() {
        let octets = alea_octets(20);
        let texte = base32(&octets);
        assert_eq!(texte.len(), 32);
        assert_eq!(base32_decode(&texte), octets);
        assert_eq!(base32_decode(""), Vec::<u8>::new());
    }

    #[test]
    fn codes_de_recuperation() {
        let codes = nouveaux_codes_recuperation(10);
        assert_eq!(codes.len(), 10);
        assert!(codes.iter().all(|c| c.len() == 9 && c.contains('-')));
        assert_eq!(empreinte_code(&codes[0]), empreinte_code(&codes[0].to_uppercase()));
        assert_ne!(empreinte_code(&codes[0]), empreinte_code(&codes[1]));
    }

    #[test]
    fn roles() {
        assert!(Role::Admin.est_admin());
        assert!(!Role::Utilisateur.est_admin());
        assert_eq!(Role::depuis_code("admin"), Role::Admin);
        assert_eq!(Role::depuis_code("autre"), Role::Utilisateur);
    }
}
