//! Génération de PDF sans dépendance (EF-FAC-05).
//!
//! Écrit un PDF 1.4 minimal : catalogue, pages, deux polices standard
//! (Helvetica, Helvetica-Bold, encodage WinAnsi pour les accents français),
//! flux de contenu texte et table `xref` exacte. Aucun outil externe, aucun
//! réseau : le PDF sort directement du binaire, ce qui garantit la
//! reproductibilité (ENF-SIMPL-01) et la portabilité (ENF-PORT-01).
//!
//! La mise en page de facture est construite dans `facture()` et couvre les
//! mentions obligatoires du CDC §11.1 : identité émetteur, client, numéro,
//! dates, désignation, quantité, prix HT, TVA, échéance, pénalités et
//! indemnité forfaitaire.

use crate::domain::*;

/// Une ligne de texte positionnée dans une page.
struct LignePdf {
    texte: String,
    taille: u32,
    gras: bool,
}

/// Document PDF en construction (les pages se créent par débordement).
pub struct Pdf {
    pages: Vec<Vec<LignePdf>>,
    hauteur: f32,
}

impl Default for Pdf {
    fn default() -> Self {
        Self::nouveau()
    }
}

impl Pdf {
    pub fn nouveau() -> Self {
        Self {
            pages: vec![Vec::new()],
            hauteur: 0.0,
        }
    }

    fn ajouter(&mut self, texte: impl Into<String>, taille: u32, gras: bool) {
        if self.pages.is_empty() {
            self.pages.push(Vec::new());
            self.hauteur = 0.0;
        }
        let interligne = taille as f32 * 1.6;
        if self.hauteur + interligne > 730.0 {
            self.pages.push(Vec::new());
            self.hauteur = 0.0;
        }
        self.hauteur += interligne;
        if let Some(page) = self.pages.last_mut() {
            page.push(LignePdf {
                texte: texte.into(),
                taille,
                gras,
            });
        }
    }

    pub fn titre(&mut self, texte: impl Into<String>) {
        self.ajouter(texte, 16, true);
    }

    pub fn sous_titre(&mut self, texte: impl Into<String>) {
        self.ajouter(texte, 12, true);
    }

    pub fn texte(&mut self, texte: impl Into<String>) {
        self.ajouter(texte, 10, false);
    }

    pub fn gras(&mut self, texte: impl Into<String>) {
        self.ajouter(texte, 10, true);
    }

    pub fn petit(&mut self, texte: impl Into<String>) {
        self.ajouter(texte, 8, false);
    }

    pub fn espace(&mut self) {
        self.ajouter(" ", 10, false);
    }

    /// Nombre de pages générées.
    pub fn nombre_pages(&self) -> usize {
        self.pages.len()
    }

    /// Sérialise le document en octets PDF.
    pub fn octets(&self) -> Vec<u8> {
        // Objets 1 (catalogue), 2 (pages), 3 (F1), 4 (F2), puis pages et flux.
        let nombre_pages = self.pages.len();
        let total_objets = 4 + nombre_pages * 2;
        let mut sortie: Vec<u8> = Vec::new();
        sortie.extend_from_slice(b"%PDF-1.4\n");
        sortie.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");
        let mut offsets: Vec<usize> = vec![0; total_objets + 1];

        let mut ecrire_objet = |numero: usize, corps: Vec<u8>| {
            offsets[numero] = sortie.len();
            sortie.extend_from_slice(format!("{numero} 0 obj\n").as_bytes());
            sortie.extend_from_slice(&corps);
            sortie.extend_from_slice(b"\nendobj\n");
        };

        ecrire_objet(1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        let kids: Vec<String> = (0..nombre_pages)
            .map(|i| format!("{} 0 R", 5 + i * 2))
            .collect();
        ecrire_objet(
            2,
            format!(
                "<< /Type /Pages /Count {} /Kids [{}] >>",
                nombre_pages,
                kids.join(" ")
            )
            .into_bytes(),
        );
        ecrire_objet(
            3,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_vec(),
        );
        ecrire_objet(
            4,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
                .to_vec(),
        );

        for (index, page) in self.pages.iter().enumerate() {
            let numero_page = 5 + index * 2;
            let numero_contenu = numero_page + 1;
            ecrire_objet(
                numero_page,
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
                     /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
                    numero_contenu
                )
                .into_bytes(),
            );
            let contenu = flux_page(page, index + 1, nombre_pages);
            let mut corps = format!("<< /Length {} >>\nstream\n", contenu.len()).into_bytes();
            corps.extend_from_slice(&contenu);
            corps.extend_from_slice(b"endstream");
            ecrire_objet(numero_contenu, corps);
        }

        let position_xref = sortie.len();
        sortie.extend_from_slice(format!("xref\n0 {}\n", total_objets + 1).as_bytes());
        sortie.extend_from_slice(b"0000000000 65535 f \n");
        for numero in 1..=total_objets {
            sortie.extend_from_slice(format!("{:010} 00000 n \n", offsets[numero]).as_bytes());
        }
        sortie.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                total_objets + 1,
                position_xref
            )
            .as_bytes(),
        );
        sortie
    }
}

fn flux_page(lignes: &[LignePdf], numero: usize, total: usize) -> Vec<u8> {
    let mut flux = String::new();
    flux.push_str("BT\n50 800 Td\n");
    for ligne in lignes {
        let police = if ligne.gras { "F2" } else { "F1" };
        let interligne = ligne.taille as f32 * 1.6;
        flux.push_str(&format!("/{police} {} Tf\n", ligne.taille));
        flux.push_str(&format!("({}) Tj\n", echapper_texte(&ligne.texte)));
        flux.push_str(&format!("0 -{:.1} Td\n", interligne));
    }
    flux.push_str(&format!(
        "/F1 8 Tf\n({}) Tj\n",
        echapper_texte(&format!(
            "{} — page {numero}/{total}",
            "Document généré par my_ERP — ERP CLI Rust"
        ))
    ));
    flux.push_str("ET\n");
    octets_latin1(&flux)
}

/// Échappe les caractères spéciaux d'une chaîne PDF.
fn echapper_texte(texte: &str) -> String {
    let mut sortie = String::new();
    for c in texte.chars() {
        match c {
            '(' => sortie.push_str("\\("),
            ')' => sortie.push_str("\\)"),
            '\\' => sortie.push_str("\\\\"),
            '\n' | '\r' => sortie.push(' '),
            c => sortie.push(c),
        }
    }
    sortie
}

/// WinAnsi = Latin-1 pour les caractères français ; le reste devient « ? ».
fn octets_latin1(texte: &str) -> Vec<u8> {
    let mut sortie = Vec::with_capacity(texte.len());
    for c in texte.chars() {
        let point = c as u32;
        if point < 256 {
            sortie.push(point as u8);
        } else {
            sortie.push(b'?');
        }
    }
    sortie
}

/// Tronque ou complète une chaîne pour une colonne de largeur fixe.
fn colonne(texte: &str, largeur: usize, droite: bool) -> String {
    let mut propre = texte.replace('\n', " ");
    let compte = propre.chars().count();
    if compte > largeur {
        propre = propre.chars().take(largeur.saturating_sub(1)).collect();
        propre.push('…');
    }
    let manque = largeur.saturating_sub(propre.chars().count());
    if droite {
        format!("{}{}", " ".repeat(manque), propre)
    } else {
        format!("{}{}", propre, " ".repeat(manque))
    }
}

/// Facture PDF complète et conforme (EF-FAC-05, CDC §11.1).
pub fn facture(societe: &Societe, client: &Client, document: &Document, regime: RegimeTva) -> Vec<u8> {
    let mut pdf = Pdf::nouveau();
    pdf.titre(societe.nom.to_uppercase());
    pdf.petit(societe.mention_identite());
    let adresse = societe.adresse_complete();
    if !adresse.is_empty() {
        pdf.petit(adresse);
    }
    if !societe.email.is_empty() || !societe.telephone.is_empty() {
        pdf.petit(format!("{} {}", societe.email, societe.telephone));
    }
    pdf.espace();
    let titre = match document.type_doc {
        TypeDocument::Devis => "DEVIS",
        TypeDocument::Facture => "FACTURE",
        TypeDocument::Avoir => "AVOIR",
    };
    pdf.titre(format!("{titre} {}", document.numero));
    pdf.texte(format!(
        "Date d'émission : {} — Prestation / livraison : {}",
        date_fr(&document.date_doc),
        date_fr(&document.date_prestation)
    ));
    pdf.texte(format!(
        "Échéance : {} — Conditions : {}",
        date_fr(&document.echeance),
        document.conditions_paiement
    ));
    if !document.devise.is_empty() && document.devise != DEVISE_REFERENCE {
        pdf.texte(format!(
            "Devise : {} (taux appliqué : 1 {} = {},{:06} EUR)",
            document.devise,
            document.devise,
            document.taux_change_micro / 1_000_000,
            document.taux_change_micro % 1_000_000
        ));
    }
    pdf.espace();
    pdf.sous_titre("Client");
    pdf.texte(client.nom.clone());
    let adresse_client = client.adresse_complete();
    if !adresse_client.is_empty() {
        pdf.texte(adresse_client);
    }
    if !client.siren.is_empty() {
        pdf.texte(format!("SIREN {}", client.siren));
    }
    if !client.tva_intracom.is_empty() {
        pdf.texte(format!("TVA intracom. {}", client.tva_intracom));
    }
    pdf.espace();
    pdf.sous_titre("Détail");
    pdf.gras(format!(
        "{}{}{}{}{}",
        colonne("Désignation", 42, false),
        colonne("Qté", 8, true),
        colonne("PU HT", 12, true),
        colonne("Rem.", 8, true),
        colonne("Total HT", 13, true)
    ));
    for ligne in &document.lignes {
        let remise = if ligne.remise_bp > 0 {
            format_taux_bp(ligne.remise_bp)
        } else {
            String::new()
        };
        pdf.texte(format!(
            "{}{}{}{}{}",
            colonne(&ligne.designation, 42, false),
            colonne(&format_quantite(ligne.quantite_milli), 8, true),
            colonne(&format_eur(ligne.prix_unitaire_ht), 12, true),
            colonne(&remise, 8, true),
            colonne(&format_eur(ligne.total_ht()), 13, true)
        ));
    }
    pdf.espace();
    let totaux = document.totaux(regime);
    if totaux.remise > 0 {
        pdf.texte(format!(
            "{}Total remises : {}",
            " ".repeat(46),
            format_eur(totaux.remise)
        ));
    }
    pdf.gras(format!(
        "{}Total HT : {}",
        " ".repeat(46),
        format_eur(totaux.ht)
    ));
    for ventilation in tva_par_taux(&document.lignes, regime) {
        if ventilation.tva == 0 && regime == RegimeTva::Franchise {
            continue;
        }
        pdf.texte(format!(
            "{}TVA {} sur {} : {}",
            " ".repeat(30),
            format_taux_bp(ventilation.taux_bp),
            format_eur(ventilation.base_ht),
            format_eur(ventilation.tva)
        ));
    }
    pdf.gras(format!(
        "{}Total TTC : {}",
        " ".repeat(46),
        format_eur(totaux.ttc)
    ));
    if document.paye != 0 {
        pdf.texte(format!(
            "{}Déjà réglé : {} — reste dû : {}",
            " ".repeat(30),
            format_eur(document.paye),
            format_eur(document.reste(regime))
        ));
    }
    pdf.espace();
    pdf.sous_titre("Mentions légales");
    if let Some(mention) = regime.mention_facture() {
        pdf.texte(mention);
    }
    pdf.petit(format!(
        "Pénalités de retard : {} par an sur le montant TTC dû, exigibles à compter de l'échéance.",
        format_taux_bp(document.penalites_taux_bp as i32)
    ));
    pdf.petit(format!(
        "Indemnité forfaitaire pour frais de recouvrement en cas de retard de paiement : {}.",
        format_eur(document.indemnite_forfaitaire)
    ));
    if !document.motif.is_empty() {
        pdf.petit(format!("Référence : {}", document.motif));
    }
    pdf.petit(format!(
        "Paiement par virement — libellé à indiquer : {} (client n°{}).",
        document.numero, document.client_id
    ));
    pdf.octets()
}

/// Compte rendu de démonstration utilisé par les tests (PDF générique).
pub fn texte_simple(lignes: &[&str]) -> Vec<u8> {
    let mut pdf = Pdf::nouveau();
    for ligne in lignes {
        pdf.texte(*ligne);
    }
    pdf.octets()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn societe() -> Societe {
        Societe {
            nom: "Dupont Conseil".into(),
            siren: "123456789".into(),
            adresse: "1 rue de la Paix".into(),
            code_postal: "06000".into(),
            ville: "Nice".into(),
            email: "contact@dupont.fr".into(),
            ..Societe::default()
        }
    }

    fn client() -> Client {
        let mut c = Client::nouveau("Atelier Dupont", TypeClient::Professionnel);
        c.adresse = "5 avenue des Fleurs".into();
        c.code_postal = "06200".into();
        c.ville = "Nice".into();
        c
    }

    fn document() -> Document {
        Document {
            numero: "FA-2026-0001".into(),
            type_doc: TypeDocument::Facture,
            date_doc: "2026-10-30".into(),
            date_prestation: "2026-10-30".into(),
            echeance: "2026-11-29".into(),
            conditions_paiement: "30 jours".into(),
            lignes: vec![
                Ligne::new("Maintenance informatique — forfait mensuel", 2.0, 50_000, 2000),
                Ligne::new("Hébergement", 1.0, 2_000, 2000).avec_remise(500),
            ],
            ..Document::default()
        }
    }

    #[test]
    fn pdf_structure_valide() {
        let octets = facture(&societe(), &client(), &document(), RegimeTva::Franchise);
        let texte = String::from_utf8_lossy(&octets).to_string();
        assert!(texte.starts_with("%PDF-1.4"));
        assert!(texte.trim_end().ends_with("%%EOF"));
        assert!(texte.contains("/Type /Catalog"));
        assert!(texte.contains("/Type /Page"));
        assert!(texte.contains("xref"));
        assert!(texte.contains("startxref"));
        // Les accents passent en WinAnsi (0xE9 = é).
        assert!(octets.iter().any(|o| *o == 0xE9));
        assert!(texte.contains("FACTURE FA-2026-0001"));
    }

    #[test]
    fn pdf_contient_les_mentions_obligatoires() {
        let octets = facture(&societe(), &client(), &document(), RegimeTva::Franchise);
        let texte = String::from_utf8_lossy(&octets).to_string();
        assert!(texte.contains("293 B du CGI"));
        assert!(texte.contains("SIREN 123456789"));
        assert!(texte.contains("Indemnit"));
        assert!(texte.contains("Total TTC"));
        assert!(texte.contains("Échéance"));
    }

    #[test]
    fn pdf_se_decoupe_en_plusieurs_pages() {
        let mut doc = document();
        for i in 0..60 {
            doc.lignes.push(Ligne::new(
                &format!("Ligne de facturation détaillée numéro {i} avec un libellé long"),
                1.0,
                1_000,
                2000,
            ));
        }
        let mut pdf = Pdf::nouveau();
        for angle in &doc.lignes {
            pdf.texte(angle.designation.clone());
        }
        assert!(pdf.nombre_pages() > 1);
        let octets = pdf.octets();
        let texte = String::from_utf8_lossy(&octets).to_string();
        let pages = texte.matches("/Type /Page ").count();
        assert_eq!(pages, pdf.nombre_pages());
        // La table xref doit annoncer tous les objets.
        assert!(texte.contains("/Count 2") || texte.contains("/Count 3"));
    }

    #[test]
    fn colonnes_alignees() {
        assert_eq!(colonne("abc", 5, false), "abc  ");
        assert_eq!(colonne("abc", 5, true), "  abc");
        assert_eq!(colonne("abcdefgh", 5, false).chars().count(), 5);
    }

    #[test]
    fn echappement_pdf() {
        assert_eq!(echapper_texte("a(b)c\\d"), "a\\(b\\)c\\\\d");
        assert_eq!(octets_latin1("é"), vec![0xE9]);
    }
}
