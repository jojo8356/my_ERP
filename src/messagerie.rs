//! Courriels : devis, factures, relances, codes de connexion (INT-01, SEC-03).
//!
//! Sans serveur SMTP dans la v1 « sans base », les messages sont écrits en
//! **boîte d'envoi locale** (`.eml` conformes RFC 5322 : en-têtes, MIME,
//! pièce jointe PDF en base64). Un agent de messagerie ou un futur connecteur
//! SMTP peut les envoyer tels quels : rien n'est perdu, tout est relu.
//!
//! Les modèles sont du texte français, sans dépendance : le but est que
//! l'utilisateur reconnaisse ses propres mots (ENF-ERG-01), pas de produire
//! du HTML.

use crate::domain::*;
use crate::pdf;
use std::path::{Path, PathBuf};

pub type Resultat<T> = Result<T, String>;

#[derive(Debug, Clone)]
pub struct Courriel {
    pub expediteur: String,
    pub destinataire: String,
    pub sujet: String,
    pub corps: String,
    /// (nom de fichier, contenu binaire) — joint en base64.
    pub pieces: Vec<(String, Vec<u8>)>,
}

impl Courriel {
    pub fn nouveau(expediteur: &str, destinataire: &str, sujet: &str, corps: String) -> Self {
        Self {
            expediteur: expediteur.to_string(),
            destinataire: destinataire.to_string(),
            sujet: sujet.to_string(),
            corps,
            pieces: Vec::new(),
        }
    }

    pub fn avec_piece(mut self, nom: &str, contenu: Vec<u8>) -> Self {
        self.pieces.push((nom.to_string(), contenu));
        self
    }
}

// ------------------------------------------------------------ Modèles

fn signature(societe: &Societe) -> String {
    let mut texte = String::from("\n—\n");
    texte.push_str(&societe.nom);
    texte.push('\n');
    if !societe.email.is_empty() {
        texte.push_str(&format!("{}\n", societe.email));
    }
    if !societe.telephone.is_empty() {
        texte.push_str(&format!("{}\n", societe.telephone));
    }
    if !societe.siren.is_empty() {
        texte.push_str(&format!("SIREN {}\n", societe.siren));
    }
    if let Some(mention) = societe.regime.mention_facture() {
        texte.push_str(&format!("{mention}\n"));
    }
    texte
}

/// Courriel d'envoi d'une facture ou d'un devis, PDF joint (EF-FAC-05).
pub fn courriel_document(
    societe: &Societe,
    client: &Client,
    document: &Document,
    regime: RegimeTva,
) -> Courriel {
    let totaux = document.totaux(regime);
    let est_devis = document.type_doc == TypeDocument::Devis;
    let sujet = format!(
        "{} {} — {}",
        if est_devis { "Devis" } else { "Facture" },
        document.numero,
        societe.nom
    );
    let mut corps = format!(
        "Bonjour {},\n\nVeuillez trouver ci-joint {} {} du {} pour un montant de {} TTC.\n",
        client.nom,
        if est_devis { "notre devis" } else { "notre facture" },
        document.numero,
        date_fr(&document.date_doc),
        format_eur(totaux.ttc)
    );
    if !est_devis {
        corps.push_str(&format!(
            "Échéance de paiement : {} ({}).\n",
            date_fr(&document.echeance),
            document.conditions_paiement
        ));
        if let Some(mention) = regime.mention_facture() {
            corps.push_str(&format!("{mention}.\n"));
        }
        corps.push_str(&format!(
            "Pénalités de retard : {} par an ; indemnité forfaitaire de recouvrement : {}.\n",
            format_taux_bp(document.penalites_taux_bp as i32),
            format_eur(document.indemnite_forfaitaire)
        ));
    }
    corps.push_str("\nBien cordialement,\n");
    corps.push_str(&signature(societe));
    Courriel::nouveau(
        &societe.email,
        &client.email,
        &sujet,
        corps,
    )
    .avec_piece(
        &format!("{}.pdf", document.numero),
        pdf::facture(societe, client, document, regime),
    )
}

/// Relance graduée (niveaux 1 à 3) avec calcul des pénalités (EF-FAC-06).
pub fn courriel_relance(
    societe: &Societe,
    client: &Client,
    document: &Document,
    regime: RegimeTva,
    niveau: u32,
    reference: &str,
) -> Courriel {
    let reste = document.reste(regime);
    let jours = jours_retard(&document.echeance, reference);
    let penalites = document.penalites(regime, reference);
    let sujet = match niveau {
        1 => format!("Rappel amical — facture {}", document.numero),
        2 => format!("Relance — facture {} en retard", document.numero),
        _ => format!("Mise en demeure — facture {} impayée", document.numero),
    };
    let mut corps = format!("Bonjour {},\n\n", client.nom);
    match niveau {
        1 => corps.push_str(&format!(
            "Sauf erreur de notre part, la facture {} du {} d'un montant de {} reste à régler.\n\
             Elle était due le {}. Il s'agit peut-être d'un simple oubli : nous vous remercions de bien vouloir procéder au règlement.\n",
            document.numero,
            date_fr(&document.date_doc),
            format_eur(reste),
            date_fr(&document.echeance)
        )),
        2 => corps.push_str(&format!(
            "Malgré notre rappel, la facture {} du {} reste impayée : {} restent dus, avec {} jour(s) de retard.\n\
             Les pénalités de retard prévues ({}) et l'indemnité forfaitaire de {} s'appliquent de plein droit.\n",
            document.numero,
            date_fr(&document.date_doc),
            format_eur(reste),
            jours,
            format_taux_bp(document.penalites_taux_bp as i32),
            format_eur(document.indemnite_forfaitaire)
        )),
        _ => corps.push_str(&format!(
            "Malgré nos relances, la facture {} demeure impayée : {} restent dus ({} jours de retard).\n\
             Le montant réclamé s'élève à {} pénalités et indemnité comprises.\n\
             Nous vous demandons de régler sous huit jours, à défaut de quoi le recouvrement sera confié à un tiers.\n",
            document.numero,
            format_eur(reste),
            jours,
            format_eur(reste + penalites)
        )),
    }
    corps.push_str("\nBien cordialement,\n");
    corps.push_str(&signature(societe));
    Courriel::nouveau(&societe.email, &client.email, &sujet, corps).avec_piece(
        &format!("{}.pdf", document.numero),
        pdf::facture(societe, client, document, regime),
    )
}

/// Code de connexion à usage unique (SEC-03) — envoyé par e-mail, 15 minutes.
///
/// Le message ne contient **que** le code et sa durée de vie : pas de lien, pas
/// de pièce jointe, rien qui puisse être rejoué après expiration.
pub fn courriel_code_connexion(
    societe: &Societe,
    utilisateur_nom: &str,
    destinataire: &str,
    code: &str,
    validite_minutes: i64,
) -> Courriel {
    let corps = format!(
        "Bonjour,\n\nVoici votre code de connexion à l'ERP de {} : {code}\n\n\
         Ce code est valable {validite_minutes} minutes et ne peut être utilisé qu'une fois.\n\
         Il remplace le code de l'application d'authentification pour cette connexion :\n\
         /auth connexion {utilisateur_nom} --mot-de-passe … --code {code}\n\n\
         Si vous n'êtes pas à l'origine de cette demande, ignorez ce message : \
         aucun code ne sera utilisable sans lui.\n",
        societe.nom
    );
    Courriel::nouveau(
        &societe.email,
        destinataire,
        &format!("Code de connexion ERP — {utilisateur_nom}"),
        corps,
    )
}

// ------------------------------------------------------------ Sérialisation MIME

fn base64(octets: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut sortie = String::new();
    for bloc in octets.chunks(3) {
        let b0 = u32::from(bloc[0]);
        let b1 = u32::from(*bloc.get(1).unwrap_or(&0));
        let b2 = u32::from(*bloc.get(2).unwrap_or(&0));
        let triplet = (b0 << 16) | (b1 << 8) | b2;
        sortie.push(ALPHABET[((triplet >> 18) & 0x3f) as usize] as char);
        sortie.push(ALPHABET[((triplet >> 12) & 0x3f) as usize] as char);
        if bloc.len() > 1 {
            sortie.push(ALPHABET[((triplet >> 6) & 0x3f) as usize] as char);
        } else {
            sortie.push('=');
        }
        if bloc.len() > 2 {
            sortie.push(ALPHABET[(triplet & 0x3f) as usize] as char);
        } else {
            sortie.push('=');
        }
    }
    sortie
}

/// Sujet encodé RFC 2047 (UTF-8 base64) : les accents passent partout.
fn sujet_encode(sujet: &str) -> String {
    if sujet.is_ascii() {
        return sujet.to_string();
    }
    format!("=?UTF-8?B?{}?=", base64(sujet.as_bytes()).replace('\n', ""))
}

fn date_rfc2822() -> String {
    let horodatage = horodatage();
    let (date, heure) = horodatage.split_once(' ').unwrap_or((&horodatage, "00:00:00"));
    let jours = jours_depuis_date(date).unwrap_or(0);
    let noms_jours = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let noms_mois = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let nom_jour = noms_jours[jour_semaine(jours) as usize];
    let nom_mois = noms_mois[(mois_de(date).saturating_sub(1) as usize).min(11)];
    format!(
        "{nom_jour}, {:02} {nom_mois} {} {heure} +0100",
        date.get(8..10).and_then(|s| s.parse::<u32>().ok()).unwrap_or(1),
        annee_de(date)
    )
}

/// Message complet prêt à être remis à un agent de messagerie.
pub fn enveloppe(courriel: &Courriel) -> String {
    let mut message = String::new();
    message.push_str(&format!("From: {}\r\n", nettoyer_entete(&courriel.expediteur)));
    message.push_str(&format!("To: {}\r\n", nettoyer_entete(&courriel.destinataire)));
    message.push_str(&format!("Subject: {}\r\n", sujet_encode(&courriel.sujet)));
    message.push_str(&format!("Date: {}\r\n", date_rfc2822()));
    message.push_str("MIME-Version: 1.0\r\n");
    message.push_str("X-Mailer: my_ERP\r\n");
    if courriel.pieces.is_empty() {
        message.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
        message.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
        message.push_str(&courriel.corps.replace('\n', "\r\n"));
        return message;
    }
    let limite = format!("=_erp_{:x}", std::process::id());
    message.push_str(&format!(
        "Content-Type: multipart/mixed; boundary=\"{limite}\"\r\n\r\n"
    ));
    message.push_str(&format!("--{limite}\r\n"));
    message.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    message.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
    message.push_str(&courriel.corps.replace('\n', "\r\n"));
    message.push_str("\r\n");
    for (nom, contenu) in &courriel.pieces {
        message.push_str(&format!("--{limite}\r\n"));
        message.push_str("Content-Type: application/pdf\r\n");
        message.push_str("Content-Transfer-Encoding: base64\r\n");
        message.push_str(&format!(
            "Content-Disposition: attachment; filename=\"{}\"\r\n\r\n",
            nom.replace('"', "")
        ));
        message.push_str(&plier(&base64(contenu), 76));
        message.push_str("\r\n");
    }
    message.push_str(&format!("--{limite}--\r\n"));
    message
}

/// Découpe en lignes de `largeur` caractères (RFC 2045 : 76 maxi).
fn plier(texte: &str, largeur: usize) -> String {
    let mut sortie = String::new();
    for morceau in texte.as_bytes().chunks(largeur) {
        sortie.push_str(&String::from_utf8_lossy(morceau));
        sortie.push_str("\r\n");
    }
    sortie
}

fn nettoyer_entete(valeur: &str) -> String {
    valeur.replace('\r', " ").replace('\n', " ").trim().to_string()
}

/// Écrit le message dans la boîte d'envoi locale et renvoie son chemin.
pub fn ecrire(dossier: &Path, courriel: &Courriel) -> Resultat<PathBuf> {
    let boite = dossier.join("courriels");
    std::fs::create_dir_all(&boite)
        .map_err(|e| format!("création de la boîte d'envoi {} : {e}", boite.display()))?;
    let nom = format!(
        "{}-{}.eml",
        horodatage().replace(':', "-").replace(' ', "_"),
        orm_empreinte(&courriel.sujet)
    );
    let chemin = boite.join(nom);
    std::fs::write(&chemin, enveloppe(courriel))
        .map_err(|e| format!("écriture de {} : {e}", chemin.display()))?;
    Ok(chemin)
}

fn orm_empreinte(texte: &str) -> String {
    crate::orm::empreinte(texte)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn societe() -> Societe {
        Societe {
            nom: "Dupont Conseil".into(),
            email: "contact@dupont.fr".into(),
            siren: "123456789".into(),
            ..Societe::default()
        }
    }

    fn client() -> Client {
        let mut c = Client::nouveau("Atelier Dupont", TypeClient::Professionnel);
        c.email = "achat@dupont.fr".into();
        c
    }

    fn facture() -> Document {
        Document {
            numero: "FA-2026-0001".into(),
            type_doc: TypeDocument::Facture,
            date_doc: "2026-10-30".into(),
            echeance: "2026-11-29".into(),
            conditions_paiement: "30 jours".into(),
            lignes: vec![Ligne::new("Maintenance", 2.0, 50_000, 2000)],
            ..Document::default()
        }
    }

    #[test]
    fn courriel_facture_complet() {
        let courriel = courriel_document(&societe(), &client(), &facture(), RegimeTva::Franchise);
        assert!(courriel.sujet.contains("FA-2026-0001"));
        // 2 × 500,00 € HT. `format_eur` emploie l'espace fine insécable.
        assert!(
            courriel.corps.contains(&format_eur(100_000)),
            "{}",
            courriel.corps
        );
        assert_eq!(courriel.pieces.len(), 1);
        let message = enveloppe(&courriel);
        assert!(message.contains("To: achat@dupont.fr"));
        assert!(message.contains("multipart/mixed"));
        assert!(message.contains("application/pdf"));
        assert!(message.contains("Content-Transfer-Encoding: base64"));
        assert!(message.starts_with("From: contact@dupont.fr"));
    }

    #[test]
    fn relances_graduees() {
        let premier = courriel_relance(&societe(), &client(), &facture(), RegimeTva::Franchise, 1, "2026-12-05");
        assert!(premier.sujet.starts_with("Rappel"));
        let second = courriel_relance(&societe(), &client(), &facture(), RegimeTva::Franchise, 2, "2026-12-20");
        assert!(second.sujet.starts_with("Relance"));
        assert!(second.corps.contains("pénalités"));
        let troisieme = courriel_relance(&societe(), &client(), &facture(), RegimeTva::Franchise, 3, "2027-02-01");
        assert!(troisieme.sujet.contains("Mise en demeure"));
        assert!(troisieme.corps.contains("recouvrement"));
    }

    #[test]
    fn base64_conforme() {
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }

    #[test]
    fn sujet_accentue_encode() {
        let encode = sujet_encode("Relance — facture");
        assert!(encode.starts_with("=?UTF-8?B?"));
        assert!(sujet_encode("Facture 42").starts_with("Facture"));
    }

    #[test]
    fn entete_sans_injection() {
        let courriel = Courriel::nouveau("a@b.fr\r\nBcc: pirate@x.fr", "c@d.fr", "Test", "corps".into());
        let message = enveloppe(&courriel);
        assert!(!message.contains("\r\nBcc:"));
    }

    #[test]
    fn courriel_de_code_de_connexion() {
        let courriel = courriel_code_connexion(
            &societe(),
            "jojo",
            "jojo@exemple.fr",
            "042731",
            15,
        );
        assert_eq!(courriel.destinataire, "jojo@exemple.fr");
        assert!(courriel.pieces.is_empty(), "aucune pièce jointe");
        assert!(courriel.sujet.contains("Code de connexion"), "{}", courriel.sujet);
        assert!(courriel.corps.contains("042731"));
        assert!(courriel.corps.contains("15 minutes"));
        assert!(courriel.corps.contains("une fois"));
        // Le message est complet et relisible tel quel (RFC 5322).
        let message = enveloppe(&courriel);
        assert!(message.contains("To: jojo@exemple.fr"));
        assert!(message.contains("Content-Type: text/plain"));
        assert!(message.contains("042731"));
        assert!(!message.contains("application/pdf"));
    }

    #[test]
    fn ecriture_dans_la_boite() {
        let dossier = std::env::temp_dir().join(format!("erp-mail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        let courriel = courriel_document(&societe(), &client(), &facture(), RegimeTva::Franchise);
        let chemin = ecrire(&dossier, &courriel).unwrap();
        assert!(chemin.exists());
        let contenu = std::fs::read_to_string(&chemin).unwrap();
        assert!(contenu.contains("Subject:"));
        let _ = std::fs::remove_dir_all(&dossier);
    }
}
