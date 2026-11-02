//! Règles métier pures de l'ERP (aucune entrée/sortie, aucune base de données).
//! Tout est testable unitairement : calculs d'argent en centimes entiers,
//! TVA (franchise en base ou TVA réelle), remises, pénalités, numérotation,
//! marges Vinted, seuils micro et DAC7, contrôle des mentions obligatoires.
//!
//! Les structures savent se décrire en texte (persistance pilotée par le
//! schéma Rustorm, ADR-006) mais ne touchent jamais au disque ni au réseau.

use std::fmt;

/// Montant en centimes (jamais de flottant pour l'argent — CDC §5.3).
pub type Cents = i64;

pub fn format_eur(cents: Cents) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{}{},{:02} €", sign, groupe_milliers(abs / 100), abs % 100)
}

/// Groupe les milliers par une espace insécable fine, à la française.
fn groupe_milliers(n: u64) -> String {
    let chiffres = n.to_string();
    let mut sortie = String::new();
    for (i, c) in chiffres.chars().enumerate() {
        if i > 0 && (chiffres.len() - i) % 3 == 0 {
            sortie.push('\u{202f}');
        }
        sortie.push(c);
    }
    sortie
}

/// Pourcentage lisible : 2000 points de base → « 20 % ».
pub fn format_taux_bp(bp: i32) -> String {
    let entier = bp / 100;
    let dec = (bp % 100).abs();
    if dec == 0 {
        format!("{entier} %")
    } else {
        format!("{entier},{dec:02} %")
    }
}

/// Pourcentage d'un plafond, borné à 999 % pour l'affichage.
pub fn pourcentage(part: Cents, total: Cents) -> i64 {
    if total <= 0 {
        return 0;
    }
    (part.saturating_mul(100) / total).min(999)
}

pub fn parse_eur(s: &str) -> Result<Cents, String> {
    let cleaned: String = s
        .trim()
        .trim_end_matches('€')
        .chars()
        .filter(|c| *c != ' ' && *c != '\u{202f}')
        .collect();
    let cleaned = cleaned.replace(',', ".");
    let (neg, cleaned) = match cleaned.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, cleaned),
    };
    let mut it = cleaned.splitn(2, '.');
    let euros: i64 = it
        .next()
        .unwrap_or("0")
        .parse()
        .map_err(|_| format!("montant illisible : « {s} »"))?;
    let cents_part = it.next().unwrap_or("");
    if cents_part.len() > 2 {
        return Err(format!("montant trop précis : « {s} »"));
    }
    let cents: i64 = format!("{:0<2}", cents_part)
        .parse()
        .map_err(|_| format!("montant illisible : « {s} »"))?;
    let total = euros * 100 + cents;
    Ok(if neg { -total } else { total })
}

/// Quantité lisible (`2500` milli → « 2,5 »).
pub fn format_quantite(milli: i64) -> String {
    let entier = milli / 1000;
    let frac = (milli % 1000).abs();
    if frac == 0 {
        format!("{entier}")
    } else {
        let mut f = format!("{frac:03}");
        while f.ends_with('0') {
            f.pop();
        }
        format!("{entier},{f}")
    }
}

// ---------------------------------------------------------------- Dates

/// Date du jour au format AAAA-MM-JJ, calculée sans dépendance externe.
pub fn aujourdhui() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    date_depuis_jours((secs / 86_400) as i64)
}

/// Horodatage « AAAA-MM-JJ HH:MM:SS » en heure de Paris (Europe/Paris, CDC §5.3).
pub fn horodatage() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let decalage = decalage_paris((secs / 86_400) as i64);
    let local = secs as i64 + decalage as i64 * 3600;
    let jours = local.div_euclid(86_400);
    let reste = local.rem_euclid(86_400);
    format!(
        "{} {:02}:{:02}:{:02}",
        date_depuis_jours(jours),
        reste / 3600,
        (reste % 3600) / 60,
        reste % 60
    )
}

/// Décalage horaire de Paris : +2 h de l'heure d'été, +1 h sinon.
fn decalage_paris(jours: i64) -> i32 {
    let annee: i64 = annee_de(&date_depuis_jours(jours)) as i64;
    // Heure d'été : du dernier dimanche de mars au dernier dimanche d'octobre.
    if jours >= dernier_dimanche(annee, 3) && jours < dernier_dimanche(annee, 10) {
        2
    } else {
        1
    }
}

fn dernier_dimanche(annee: i64, mois: u32) -> i64 {
    let mut jour = jours_du_mois(annee as u32, mois) as i64;
    while jour > 0 {
        if let Some(j) = jours_depuis_date(&format!("{annee}-{mois:02}-{jour:02}")) {
            if jour_semaine(j) == 6 {
                return j;
            }
        }
        jour -= 1;
    }
    jours_depuis_date(&format!("{annee}-{mois:02}-01")).unwrap_or(0)
}

/// Jour de la semaine : 0 = lundi … 6 = dimanche.
pub fn jour_semaine(jours: i64) -> i32 {
    // 1970-01-01 était un jeudi (= 3 en partant de lundi).
    ((jours + 3).rem_euclid(7)) as i32
}

/// Conversion jours depuis 1970-01-01 -> AAAA-MM-JJ (algorithme civil de H. Hinnant).
pub fn date_depuis_jours(z: i64) -> String {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Conversion AAAA-MM-JJ -> jours depuis 1970-01-01.
pub fn jours_depuis_date(date: &str) -> Option<i64> {
    let (a, m, j) = decouper_date(date)?;
    let y = if m <= 2 { a as i64 - 1 } else { a as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m as i64 + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + j as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn decouper_date(date: &str) -> Option<(u32, u32, u32)> {
    let mut it = date.split('-');
    let a: u32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let j: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || j == 0 || j > jours_du_mois(a, m) {
        return None;
    }
    Some((a, m, j))
}

pub fn jours_du_mois(annee: u32, mois: u32) -> u32 {
    match mois {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if annee % 4 == 0 && (annee % 100 != 0 || annee % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

pub fn date_valide(date: &str) -> bool {
    decouper_date(date).is_some()
}

pub fn ajouter_jours(date: &str, jours: i64) -> String {
    match jours_depuis_date(date) {
        Some(j) => date_depuis_jours(j + jours),
        None => date.to_string(),
    }
}

/// Nombre de jours de `debut` à `fin` (négatif si `fin` précède `debut`).
pub fn jours_entre(debut: &str, fin: &str) -> i64 {
    match (jours_depuis_date(debut), jours_depuis_date(fin)) {
        (Some(a), Some(b)) => b - a,
        _ => 0,
    }
}

/// « 2026-10-30 » → « 30/10/2026 ».
pub fn date_fr(date: &str) -> String {
    match decouper_date(date) {
        Some((a, m, j)) => format!("{j:02}/{m:02}/{a:04}"),
        None => date.to_string(),
    }
}

pub fn annee_de(date: &str) -> u32 {
    date.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(2026)
}

pub fn mois_de(date: &str) -> u32 {
    date.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Période mensuelle « AAAA-MM » d'une date.
pub fn periode_de(date: &str) -> String {
    date.get(0..7).unwrap_or("").to_string()
}

// ---------------------------------------------------------------- TVA

/// Régime de TVA de la société (CDC §21.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegimeTva {
    /// Micro en franchise en base : pas de TVA facturée (art. 293 B CGI).
    Franchise,
    /// TVA réelle. Pour les services : exigible à l'encaissement (défaut légal).
    Reel,
}

impl RegimeTva {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "franchise" => Ok(Self::Franchise),
            "reel" | "réel" => Ok(Self::Reel),
            _ => Err(format!("régime TVA inconnu : « {s} » (franchise|reel)")),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Franchise => "franchise",
            Self::Reel => "reel",
        }
    }
    pub fn libelle(&self) -> &'static str {
        match self {
            Self::Franchise => "franchise en base",
            Self::Reel => "TVA réelle",
        }
    }
    pub fn mention_facture(&self) -> Option<&'static str> {
        match self {
            Self::Franchise => Some("TVA non applicable, art. 293 B du CGI"),
            Self::Reel => None,
        }
    }
}

/// Une ligne de devis/facture. `tva_bp` = taux en points de base (20 % = 2000).
///
/// `id`, `document_id`, `position` et `article_id` sont des informations de
/// persistance (le schéma Rustorm les déclare) : elles ne participent à aucun
/// calcul, ce qui garde les règles métier testables isolément.
#[derive(Debug, Clone, PartialEq)]
pub struct Ligne {
    pub id: i64,
    pub document_id: i64,
    pub position: i64,
    pub designation: String,
    pub quantite_milli: i64,
    pub prix_unitaire_ht: Cents,
    /// Remise en points de base sur la ligne (1000 = 10 %).
    pub remise_bp: i32,
    pub tva_bp: i32,
    pub article_id: i64,
}

impl Default for Ligne {
    fn default() -> Self {
        Self {
            id: 0,
            document_id: 0,
            position: 0,
            designation: String::new(),
            quantite_milli: 1000,
            prix_unitaire_ht: 0,
            remise_bp: 0,
            tva_bp: 2000,
            article_id: 0,
        }
    }
}

impl Ligne {
    pub fn new(designation: &str, quantite: f64, prix_unitaire_ht: Cents, tva_bp: i32) -> Self {
        Self {
            designation: designation.to_string(),
            quantite_milli: (quantite * 1000.0).round() as i64,
            prix_unitaire_ht,
            tva_bp,
            ..Self::default()
        }
    }

    pub fn avec_remise(mut self, remise_bp: i32) -> Self {
        self.remise_bp = remise_bp.clamp(0, 10_000);
        self
    }

    pub fn avec_article(mut self, article_id: i64) -> Self {
        self.article_id = article_id;
        self
    }

    /// Total HT de la ligne, remise déduite.
    pub fn total_ht(&self) -> Cents {
        let brut = (self.quantite_milli * self.prix_unitaire_ht + 500) / 1000;
        let remise = self.montant_remise();
        brut - remise
    }

    /// Montant de la remise (arrondi au centime).
    pub fn montant_remise(&self) -> Cents {
        if self.remise_bp <= 0 {
            return 0;
        }
        let brut = (self.quantite_milli * self.prix_unitaire_ht + 500) / 1000;
        (brut * i64::from(self.remise_bp) + 5000) / 10_000
    }

    pub fn total_tva(&self, regime: RegimeTva, taux_effectif_bp: i32) -> Cents {
        match regime {
            RegimeTva::Franchise => 0,
            RegimeTva::Reel => (self.total_ht() * i64::from(taux_effectif_bp) + 5000) / 10_000,
        }
    }

    /// Taux réellement appliqué à la ligne.
    pub fn tva_effective_bp(&self) -> i32 {
        self.tva_bp
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Totaux {
    pub ht: Cents,
    pub tva: Cents,
    pub ttc: Cents,
    pub remise: Cents,
}

pub fn totaux(lignes: &[Ligne], regime: RegimeTva) -> Totaux {
    let ht: Cents = lignes.iter().map(|l| l.total_ht()).sum();
    let tva: Cents = lignes
        .iter()
        .map(|l| l.total_tva(regime, l.tva_effective_bp()))
        .sum();
    let remise: Cents = lignes.iter().map(|l| l.montant_remise()).sum();
    Totaux {
        ht,
        tva,
        ttc: ht + tva,
        remise,
    }
}

/// Ventilation de la TVA par taux (EF-CPT-02 : récapitulatif par taux).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TvaParTaux {
    pub taux_bp: i32,
    pub base_ht: Cents,
    pub tva: Cents,
}

pub fn tva_par_taux(lignes: &[Ligne], regime: RegimeTva) -> Vec<TvaParTaux> {
    let mut out: Vec<TvaParTaux> = Vec::new();
    for ligne in lignes {
        let taux_bp = ligne.tva_effective_bp();
        let idx = match out.iter().position(|t| t.taux_bp == taux_bp) {
            Some(i) => i,
            None => {
                out.push(TvaParTaux {
                    taux_bp,
                    base_ht: 0,
                    tva: 0,
                });
                out.len() - 1
            }
        };
        out[idx].base_ht += ligne.total_ht();
        out[idx].tva += ligne.total_tva(regime, taux_bp);
    }
    out.sort_by_key(|t| t.taux_bp);
    out
}

// ---------------------------------------------------------------- Documents

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeDocument {
    Devis,
    Facture,
    Avoir,
}

impl TypeDocument {
    pub fn prefixe(&self) -> &'static str {
        match self {
            Self::Devis => "DEV",
            Self::Facture => "FA",
            Self::Avoir => "AV",
        }
    }
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "devis" => Ok(Self::Devis),
            "facture" => Ok(Self::Facture),
            "avoir" => Ok(Self::Avoir),
            _ => Err(format!("type de document inconnu : « {s} »")),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Devis => "devis",
            Self::Facture => "facture",
            Self::Avoir => "avoir",
        }
    }
    /// Un devis peut expirer ; une facture non.
    pub fn expire(&self) -> bool {
        matches!(self, Self::Devis)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatutDocument {
    Brouillon,
    Envoye,
    Accepte,
    Refuse,
    Expire,
    Valide,
    Paye,
    PartiellementPaye,
    Annule,
}

impl StatutDocument {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Brouillon => "brouillon",
            Self::Envoye => "envoyé",
            Self::Accepte => "accepté",
            Self::Refuse => "refusé",
            Self::Expire => "expiré",
            Self::Valide => "validée",
            Self::Paye => "payée",
            Self::PartiellementPaye => "partiellement payée",
            Self::Annule => "annulée",
        }
    }
    /// Code stable pour la persistance et les échanges (ASCII, sans accent).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Brouillon => "brouillon",
            Self::Envoye => "envoye",
            Self::Accepte => "accepte",
            Self::Refuse => "refuse",
            Self::Expire => "expire",
            Self::Valide => "valide",
            Self::Paye => "paye",
            Self::PartiellementPaye => "partiel",
            Self::Annule => "annule",
        }
    }
    pub fn depuis_code(code: &str) -> Self {
        match code.trim().to_lowercase().as_str() {
            "envoye" => Self::Envoye,
            "accepte" => Self::Accepte,
            "refuse" => Self::Refuse,
            "expire" => Self::Expire,
            "valide" => Self::Valide,
            "paye" => Self::Paye,
            "partiel" => Self::PartiellementPaye,
            "annule" => Self::Annule,
            _ => Self::Brouillon,
        }
    }
    pub fn symbole(&self) -> &'static str {
        match self {
            Self::Paye | Self::Accepte => "✓",
            Self::Brouillon => "…",
            Self::Refuse | Self::Annule | Self::Expire => "✗",
            _ => "!",
        }
    }
    /// États terminaux : plus aucune transition métier.
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Paye | Self::Annule | Self::Refuse | Self::Expire | Self::Accepte
        )
    }
}

/// Numéro officiel : préfixe-année-séquence sur 4 chiffres (FA-2026-0042).
pub fn numero_document(t: TypeDocument, annee: u32, seq: u32) -> String {
    format!("{}-{}-{:04}", t.prefixe(), annee, seq)
}

/// Une facture validée est immuable : seule la création d'un avoir corrige (CDC EF-FAC-02).
pub fn peut_modifier(statut: StatutDocument) -> bool {
    matches!(statut, StatutDocument::Brouillon)
}

pub fn reste_a_payer(total_ttc: Cents, deja_paye: Cents) -> Cents {
    (total_ttc - deja_paye).max(0)
}

pub fn statut_apres_paiement(total_ttc: Cents, deja_paye: Cents) -> StatutDocument {
    if deja_paye <= 0 {
        StatutDocument::Valide
    } else if deja_paye < total_ttc {
        StatutDocument::PartiellementPaye
    } else {
        StatutDocument::Paye
    }
}

/// Jours de retard d'un document (0 si l'échéance n'est pas dépassée).
pub fn jours_retard(echeance: &str, reference: &str) -> i64 {
    let retard = jours_entre(echeance, reference);
    retard.max(0)
}

/// Pénalités de retard : montant TTC × taux annuel × jours / 365 (EF-FAC-04).
pub fn penalites_retard(montant_ttc: Cents, jours: i64, taux_bp: i64) -> Cents {
    if jours <= 0 || taux_bp <= 0 || montant_ttc <= 0 {
        return 0;
    }
    (montant_ttc * taux_bp * jours + 365 * 5000) / (365 * 10_000)
}

/// Indemnité forfaitaire pour frais de recouvrement (40 € par défaut, B2B).
pub const INDEMNITE_FORFAITAIRE: Cents = 4_000;

// ---------------------------------------------------------------- Contrôle des mentions (CDC §11.1)

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControleMentions {
    pub manquants: Vec<String>,
}

impl ControleMentions {
    pub fn conforme(&self) -> bool {
        self.manquants.is_empty()
    }
}

/// Vérifie les mentions obligatoires d'une facture avant validation (EF-FAC-02).
pub fn controle_mentions(societe: &Societe, client: &Client, doc: &Document) -> ControleMentions {
    let mut manquants = Vec::new();
    let mut exiger = |condition: bool, message: &str| {
        if !condition {
            manquants.push(message.to_string());
        }
    };
    exiger(!societe.nom.trim().is_empty(), "dénomination de l'émetteur");
    exiger(!societe.siren.trim().is_empty(), "SIREN de l'émetteur");
    exiger(!societe.adresse.trim().is_empty(), "adresse de l'émetteur");
    exiger(!societe.ville.trim().is_empty(), "ville de l'émetteur");
    exiger(
        !client.nom.trim().is_empty(),
        "identité du client (nom ou raison sociale)",
    );
    exiger(
        !doc.numero.trim().is_empty(),
        "numéro unique chronologique",
    );
    exiger(date_valide(&doc.date_doc), "date d'émission valide");
    exiger(
        date_valide(&doc.date_prestation) || date_valide(&doc.date_doc),
        "date de la prestation ou de la livraison",
    );
    exiger(!doc.lignes.is_empty(), "au moins une ligne de facturation");
    exiger(
        doc.lignes.iter().all(|l| !l.designation.trim().is_empty()),
        "désignation précise de chaque ligne",
    );
    exiger(
        doc.lignes.iter().all(|l| l.prix_unitaire_ht != 0),
        "prix unitaire HT sur chaque ligne",
    );
    exiger(date_valide(&doc.echeance), "échéance de paiement");
    exiger(
        doc.penalites_taux_bp > 0,
        "taux des pénalités de retard",
    );
    exiger(
        doc.indemnite_forfaitaire > 0,
        "indemnité forfaitaire pour frais de recouvrement",
    );
    if matches!(doc.type_doc, TypeDocument::Facture | TypeDocument::Avoir)
        && societe.regime == RegimeTva::Reel
    {
        exiger(
            doc.lignes.iter().any(|l| l.tva_effective_bp() > 0),
            "taux de TVA par ligne (régime réel)",
        );
    }
    ControleMentions { manquants }
}

// ---------------------------------------------------------------- Clients

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeClient {
    Particulier,
    Professionnel,
    Association,
}

impl TypeClient {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "particulier" | "part" => Ok(Self::Particulier),
            "pro" | "professionnel" => Ok(Self::Professionnel),
            "asso" | "association" => Ok(Self::Association),
            _ => Err(format!(
                "type de client inconnu : « {s} » (pro|particulier|association)"
            )),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Particulier => "particulier",
            Self::Professionnel => "professionnel",
            Self::Association => "association",
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::Particulier => "particulier",
            Self::Professionnel => "professionnel",
            Self::Association => "association",
        }
    }
    pub fn depuis_code(code: &str) -> Self {
        match code {
            "particulier" => Self::Particulier,
            "association" => Self::Association,
            _ => Self::Professionnel,
        }
    }
    /// Un professionnel exige une facture ; un particulier seulement ≥ 25 € (CDC §11.2).
    pub fn facture_obligatoire(&self, montant_ttc: Cents) -> bool {
        match self {
            Self::Particulier => montant_ttc >= 2_500,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Client {
    pub id: i64,
    pub societe_id: i64,
    pub nom: String,
    pub type_client: TypeClient,
    pub email: String,
    pub telephone: String,
    pub adresse: String,
    pub code_postal: String,
    pub ville: String,
    pub pays: String,
    pub siren: String,
    pub tva_intracom: String,
    pub notes: String,
    pub etiquettes: Vec<String>,
    pub reference_externe: String,
    /// Date d'effacement RGPD (anonymisation) — vide tant que le client est actif.
    pub rgpd_efface_le: String,
    pub cree_le: String,
    pub modifie_le: String,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            id: 0,
            societe_id: 0,
            nom: String::new(),
            type_client: TypeClient::Professionnel,
            email: String::new(),
            telephone: String::new(),
            adresse: String::new(),
            code_postal: String::new(),
            ville: String::new(),
            pays: "FR".to_string(),
            siren: String::new(),
            tva_intracom: String::new(),
            notes: String::new(),
            etiquettes: Vec::new(),
            reference_externe: String::new(),
            rgpd_efface_le: String::new(),
            cree_le: String::new(),
            modifie_le: String::new(),
        }
    }
}

impl Client {
    pub fn nouveau(nom: &str, type_client: TypeClient) -> Self {
        Self {
            nom: nom.to_string(),
            type_client,
            ..Self::default()
        }
    }
    pub fn etiquette_presente(&self, etiquette: &str) -> bool {
        self.etiquettes
            .iter()
            .any(|e| e.eq_ignore_ascii_case(etiquette))
    }
    pub fn adresse_complete(&self) -> String {
        let mut parts = Vec::new();
        if !self.adresse.trim().is_empty() {
            parts.push(self.adresse.trim().to_string());
        }
        let ligne_ville = format!("{} {}", self.code_postal.trim(), self.ville.trim())
            .trim()
            .to_string();
        if !ligne_ville.is_empty() {
            parts.push(ligne_ville);
        }
        parts.join(", ")
    }
}

impl fmt::Display for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.nom, self.type_client.as_str())
    }
}

// ---------------------------------------------------------------- Société

/// Société (ou activité juridique) — le multi-société du CDC §21.6.
#[derive(Debug, Clone, PartialEq)]
pub struct Societe {
    pub id: i64,
    pub nom: String,
    pub forme_juridique: String,
    pub capital: String,
    pub siren: String,
    pub siret: String,
    pub tva_intracom: String,
    pub adresse: String,
    pub code_postal: String,
    pub ville: String,
    pub pays: String,
    pub email: String,
    pub telephone: String,
    pub regime: RegimeTva,
    pub devise: String,
    pub delai_paiement_jours: i64,
    pub penalites_taux_bp: i64,
    pub indemnite_forfaitaire: Cents,
    pub cree_le: String,
}

impl Default for Societe {
    fn default() -> Self {
        Self {
            id: 1,
            nom: "Ma micro-entreprise".to_string(),
            forme_juridique: "micro-entreprise".to_string(),
            capital: String::new(),
            siren: String::new(),
            siret: String::new(),
            tva_intracom: String::new(),
            adresse: String::new(),
            code_postal: String::new(),
            ville: String::new(),
            pays: "FR".to_string(),
            email: String::new(),
            telephone: String::new(),
            regime: RegimeTva::Franchise,
            devise: "EUR".to_string(),
            delai_paiement_jours: 30,
            penalites_taux_bp: 1_000,
            indemnite_forfaitaire: INDEMNITE_FORFAITAIRE,
            cree_le: String::new(),
        }
    }
}

impl Societe {
    pub fn conditions_paiement(&self) -> String {
        format!("{} jours", self.delai_paiement_jours)
    }
    pub fn adresse_complete(&self) -> String {
        let mut parts = Vec::new();
        if !self.adresse.trim().is_empty() {
            parts.push(self.adresse.trim().to_string());
        }
        let ligne_ville = format!("{} {}", self.code_postal.trim(), self.ville.trim())
            .trim()
            .to_string();
        if !ligne_ville.is_empty() {
            parts.push(ligne_ville);
        }
        parts.join(", ")
    }
    /// Identité complète de l'émetteur sur une facture (CDC §11.1).
    pub fn mention_identite(&self) -> String {
        let mut bout = vec![format!("{} ({})", self.nom, self.forme_juridique)];
        if !self.capital.trim().is_empty() {
            bout.push(format!("capital {}", self.capital));
        }
        if !self.siren.trim().is_empty() {
            bout.push(format!("SIREN {}", self.siren));
        }
        if !self.siret.trim().is_empty() {
            bout.push(format!("SIRET {}", self.siret));
        }
        if !self.tva_intracom.trim().is_empty() {
            bout.push(format!("TVA intracom. {}", self.tva_intracom));
        }
        bout.join(" — ")
    }
}

// ---------------------------------------------------------------- Devises (EF-SOC-02)

/// Taux de change daté : 1 unité de `devise` = `taux_micro` / 1 000 000 EUR.
#[derive(Debug, Clone, PartialEq)]
pub struct TauxChange {
    pub id: i64,
    pub devise: String,
    pub date_taux: String,
    pub taux_micro: i64,
    pub source: String,
}

impl TauxChange {
    pub fn libelle(&self) -> String {
        format!(
            "1 {} = {},{:06} EUR",
            self.devise,
            self.taux_micro / 1_000_000,
            self.taux_micro % 1_000_000
        )
    }
}

/// Convertit un montant d'une devise vers la devise de référence.
pub fn convertir(cents: Cents, taux_micro: i64) -> Cents {
    if taux_micro <= 0 {
        return cents;
    }
    (cents * taux_micro + 500_000) / 1_000_000
}

pub fn parse_taux(texte: &str) -> Result<i64, String> {
    let euros = parse_eur(texte)?;
    Ok(euros.saturating_mul(10_000))
}

// ---------------------------------------------------------------- Vinted / vêtements

/// Régime fiscal d'un article (CDC §11.3, EF-VIN-02) — jamais mélangés.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegimeVente {
    /// Achat en vue de revendre : activité commerciale (BIC) dès le premier euro.
    AchatRevente,
    /// Bien personnel d'occasion : en principe non imposable.
    Personnel,
}

impl RegimeVente {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "achat_revente" | "achat-revente" | "pro" | "bic" => Ok(Self::AchatRevente),
            "personnel" | "perso" | "occasion" => Ok(Self::Personnel),
            _ => Err(format!(
                "régime inconnu : « {s} » (achat_revente|personnel)"
            )),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AchatRevente => "achat_revente",
            Self::Personnel => "personnel",
        }
    }
    pub fn libelle(&self) -> &'static str {
        match self {
            Self::AchatRevente => "achat-revente (BIC)",
            Self::Personnel => "bien personnel (non imposable)",
        }
    }
    pub fn compte_vente(&self) -> &'static str {
        match self {
            Self::AchatRevente => "707000",
            Self::Personnel => "708500",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Article {
    pub id: i64,
    pub societe_id: i64,
    pub marque: String,
    pub description: String,
    pub taille: String,
    pub composition: String, // obligatoire UE 1007/2011 (CDC §11.3)
    pub etat: String,
    pub prix_achat: Cents,
    pub date_achat: String,
    pub statut: String,
    pub regime: RegimeVente,
    pub photos: Vec<String>,
    pub sku: String,
    pub quantite: i64,
    pub cree_le: String,
}

impl Default for Article {
    fn default() -> Self {
        Self {
            id: 0,
            societe_id: 0,
            marque: String::new(),
            description: String::new(),
            taille: String::new(),
            composition: String::new(),
            etat: "très bon".to_string(),
            prix_achat: 0,
            date_achat: String::new(),
            statut: "en_stock".to_string(),
            regime: RegimeVente::AchatRevente,
            photos: Vec::new(),
            sku: String::new(),
            quantite: 1,
            cree_le: String::new(),
        }
    }
}

impl Article {
    pub fn nouveau(marque: &str, description: &str) -> Self {
        Self {
            marque: marque.to_string(),
            description: description.to_string(),
            ..Self::default()
        }
    }
    pub fn en_stock(&self) -> bool {
        self.statut != "vendu"
    }
    /// Fiche produit exportable, prête à coller dans une annonce (EF-VIN-05).
    pub fn annonce(&self) -> String {
        let mut texte = String::new();
        texte.push_str(&format!("{}\n", self.description));
        if !self.marque.is_empty() {
            texte.push_str(&format!("Marque : {}\n", self.marque));
        }
        if !self.taille.is_empty() {
            texte.push_str(&format!("Taille : {}\n", self.taille));
        }
        texte.push_str(&format!("État : {}\n", self.etat));
        texte.push_str(&format!("Composition : {}\n", self.composition));
        texte.push_str(&format!("Prix : {}\n", format_eur(self.prix_vente_conseille())));
        texte
    }
    /// Prix conseillé : prix d'achat majoré de 150 % (arrondi au demi-euro).
    pub fn prix_vente_conseille(&self) -> Cents {
        let brut = self.prix_achat * 5 / 2;
        ((brut + 50) / 100) * 100
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Vente {
    pub id: i64,
    pub societe_id: i64,
    pub article_id: i64,
    pub date_vente: String,
    pub prix_vente: Cents,
    pub frais: Cents, // plateforme + port à charge du vendeur
    pub plateforme: String,
    pub acheteur: String,
    pub reference: String,
    pub regime: RegimeVente,
    pub cree_le: String,
}

impl Default for Vente {
    fn default() -> Self {
        Self {
            id: 0,
            societe_id: 0,
            article_id: 0,
            date_vente: String::new(),
            prix_vente: 0,
            frais: 0,
            plateforme: "Vinted".to_string(),
            acheteur: String::new(),
            reference: String::new(),
            regime: RegimeVente::AchatRevente,
            cree_le: String::new(),
        }
    }
}

impl Vente {
    pub fn nouvelle(article_id: i64, prix_vente: Cents, frais: Cents) -> Self {
        Self {
            article_id,
            prix_vente,
            frais,
            ..Self::default()
        }
    }
}

/// Marge nette d'une vente d'achat-revente (CDC EF-VIN-02).
pub fn marge_nette(article: &Article, vente: &Vente) -> Cents {
    vente.prix_vente - vente.frais - article.prix_achat
}

/// Marge en pourcentage du prix de vente (0 si vente nulle).
pub fn marge_pourcentage(article: &Article, vente: &Vente) -> i64 {
    if vente.prix_vente <= 0 {
        return 0;
    }
    marge_nette(article, vente) * 100 / vente.prix_vente
}

/// Seuils DAC7 : signalement plateforme (30 ventes OU 2 000 €) — pas une imposition.
pub fn alerte_dac7(nb_ventes: u32, ca: Cents) -> Option<String> {
    if nb_ventes >= 30 || ca >= 200_000 {
        Some(format!(
            "seuil DAC7 franchi ({nb_ventes} ventes, {}) : la plateforme transmettra vos données au fisc (signalement, pas imposition automatique)",
            format_eur(ca)
        ))
    } else if nb_ventes >= 25 || ca >= 170_000 {
        Some(format!(
            "approche du seuil DAC7 ({nb_ventes}/30 ventes, {} / 2 000,00 €)",
            format_eur(ca)
        ))
    } else {
        None
    }
}

/// Seuils 2026 du régime micro en activité mixte (CDC §21.1) + franchise en base TVA.
#[derive(Debug, Clone, Copy)]
pub struct SeuilsMicro {
    pub ca_global_max: Cents,
    pub ca_services_max: Cents,
    pub franchise_tva_services: Cents,
    pub franchise_tva_ventes: Cents,
    pub majoration_bp: i64,
}

pub const SEUILS_2026: SeuilsMicro = SeuilsMicro {
    ca_global_max: 203_100 * 100,
    ca_services_max: 83_600 * 100,
    franchise_tva_services: 37_500 * 100,
    franchise_tva_ventes: 85_000 * 100,
    majoration_bp: 1_100,
};

#[derive(Debug, Clone, PartialEq)]
pub struct EtatSeuils {
    pub ca_ventes: Cents,
    pub ca_services: Cents,
    pub alertes: Vec<String>,
}

impl EtatSeuils {
    pub fn ca_global(&self) -> Cents {
        self.ca_ventes + self.ca_services
    }
    pub fn pct_global(&self) -> i64 {
        pourcentage(self.ca_global(), SEUILS_2026.ca_global_max)
    }
    pub fn pct_services(&self) -> i64 {
        pourcentage(self.ca_services, SEUILS_2026.ca_services_max)
    }
    pub fn pct_franchise_services(&self) -> i64 {
        pourcentage(self.ca_services, SEUILS_2026.franchise_tva_services)
    }
    pub fn pct_franchise_ventes(&self) -> i64 {
        pourcentage(self.ca_ventes, SEUILS_2026.franchise_tva_ventes)
    }
}

pub fn etat_seuils(ca_ventes: Cents, ca_services: Cents, seuils: SeuilsMicro) -> EtatSeuils {
    let mut alertes = Vec::new();
    let global = ca_ventes + ca_services;
    if global > seuils.ca_global_max {
        alertes.push(format!(
            "CA global {} au-delà du plafond micro ({}) : sortie du régime micro",
            format_eur(global),
            format_eur(seuils.ca_global_max)
        ));
    } else if global * 100 > seuils.ca_global_max * 90 {
        alertes.push(format!(
            "CA global {} à {} % du plafond micro ({})",
            format_eur(global),
            pourcentage(global, seuils.ca_global_max),
            format_eur(seuils.ca_global_max)
        ));
    }
    if ca_services > seuils.ca_services_max {
        alertes.push(format!(
            "CA services {} au-delà du plafond services ({})",
            format_eur(ca_services),
            format_eur(seuils.ca_services_max)
        ));
    }
    if ca_services > seuils.franchise_tva_services {
        alertes.push(format!(
            "CA services {} au-delà du seuil de franchise TVA services ({}) : vérifier l'assujettissement TVA",
            format_eur(ca_services),
            format_eur(seuils.franchise_tva_services)
        ));
    }
    if ca_ventes > seuils.franchise_tva_ventes {
        alertes.push(format!(
            "CA ventes {} au-delà du seuil de franchise TVA ventes ({}) : vérifier l'assujettissement TVA",
            format_eur(ca_ventes),
            format_eur(seuils.franchise_tva_ventes)
        ));
    }
    EtatSeuils {
        ca_ventes,
        ca_services,
        alertes,
    }
}

// ---------------------------------------------------------------- Documents (structure complète)

/// Devise de référence de l'ERP (les taux sont exprimés vers l'euro).
pub const DEVISE_REFERENCE: &str = "EUR";

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub id: i64,
    pub societe_id: i64,
    pub numero: String,
    pub type_doc: TypeDocument,
    pub statut: StatutDocument,
    pub client_id: i64,
    pub date_doc: String,
    pub date_prestation: String,
    pub echeance: String,
    pub activite: String, // "services" | "ventes"
    pub devise: String,
    /// 1 devise = taux_change_micro / 1 000 000 EUR.
    pub taux_change_micro: i64,
    pub conditions_paiement: String,
    pub delai_paiement_jours: i64,
    pub penalites_taux_bp: i64,
    pub indemnite_forfaitaire: Cents,
    pub lignes: Vec<Ligne>,
    pub paye: Cents,
    pub envoye_le: String,
    pub motif: String,
    pub cree_le: String,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            id: 0,
            societe_id: 0,
            numero: String::new(),
            type_doc: TypeDocument::Facture,
            statut: StatutDocument::Brouillon,
            client_id: 0,
            date_doc: String::new(),
            date_prestation: String::new(),
            echeance: String::new(),
            activite: "services".to_string(),
            devise: DEVISE_REFERENCE.to_string(),
            taux_change_micro: 1_000_000,
            conditions_paiement: String::new(),
            delai_paiement_jours: 30,
            penalites_taux_bp: 1_000,
            indemnite_forfaitaire: INDEMNITE_FORFAITAIRE,
            lignes: Vec::new(),
            paye: 0,
            envoye_le: String::new(),
            motif: String::new(),
            cree_le: String::new(),
        }
    }
}

impl Document {
    pub fn totaux(&self, regime: RegimeTva) -> Totaux {
        totaux(&self.lignes, regime)
    }
    /// Totaux convertis dans la devise de la société (EF-SOC-02).
    pub fn totaux_reference(&self, regime: RegimeTva) -> Totaux {
        let t = self.totaux(regime);
        if self.devise == DEVISE_REFERENCE {
            return t;
        }
        Totaux {
            ht: convertir(t.ht, self.taux_change_micro),
            tva: convertir(t.tva, self.taux_change_micro),
            ttc: convertir(t.ttc, self.taux_change_micro),
            remise: convertir(t.remise, self.taux_change_micro),
        }
    }
    pub fn reste(&self, regime: RegimeTva) -> Cents {
        reste_a_payer(self.totaux(regime).ttc, self.paye)
    }
    pub fn en_retard(&self, reference: &str) -> bool {
        matches!(
            self.statut,
            StatutDocument::Valide | StatutDocument::PartiellementPaye
        ) && jours_retard(&self.echeance, reference) > 0
    }
    pub fn est_impaye(&self) -> bool {
        matches!(
            self.statut,
            StatutDocument::Valide | StatutDocument::PartiellementPaye
        )
    }
    pub fn penalites(&self, regime: RegimeTva, reference: &str) -> Cents {
        let jours = jours_retard(&self.echeance, reference);
        if !self.est_impaye() || jours == 0 {
            return 0;
        }
        let assiette = self.reste(regime);
        penalites_retard(assiette, jours, self.penalites_taux_bp)
            + self.indemnite_forfaitaire
    }
    /// Niveau de relance recommandé (0 = rien à faire).
    pub fn niveau_relance(&self, reference: &str) -> u32 {
        if !self.est_impaye() {
            return 0;
        }
        let jours = jours_retard(&self.echeance, reference);
        if jours <= 0 {
            0
        } else if jours <= 15 {
            1
        } else if jours <= 30 {
            2
        } else {
            3
        }
    }
}

// ---------------------------------------------------------------- Divers

pub fn annee_courante() -> u32 {
    annee_de(&aujourdhui())
}

pub fn slug_recherche(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'à' | 'â' | 'ä' => 'a',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'œ' => 'o',
            _ => c,
        })
        .collect()
}

/// Recherche floue simple : toutes les parties cherchées apparaissent (insensible casse/accents).
pub fn correspond(haystack: &str, needle: &str) -> bool {
    let h = slug_recherche(haystack);
    let needle = slug_recherche(needle);
    if needle.trim().is_empty() {
        return true;
    }
    needle.split_whitespace().all(|part| h.contains(part))
}

/// Liste d'étiquettes ↔ texte persisté (séparateur « , »).
pub fn etiquettes_vers_texte(etiquettes: &[String]) -> String {
    etiquettes
        .iter()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn etiquettes_depuis_texte(texte: &str) -> Vec<String> {
    texte
        .split(',')
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_et_parse_eur() {
        assert_eq!(format_eur(0), "0,00 €");
        assert_eq!(format_eur(4_250), "42,50 €");
        assert_eq!(format_eur(123_456), "1\u{202f}234,56 €");
        assert_eq!(format_eur(-500), "-5,00 €");
        assert_eq!(parse_eur("42,50").unwrap(), 4_250);
        assert_eq!(parse_eur("1 234,56 €").unwrap(), 123_456);
        assert_eq!(parse_eur("100").unwrap(), 10_000);
        assert!(parse_eur("abc").is_err());
    }

    #[test]
    fn quantites_et_taux() {
        assert_eq!(format_quantite(2_500), "2,5");
        assert_eq!(format_quantite(3_000), "3");
        assert_eq!(format_taux_bp(2_000), "20 %");
        assert_eq!(format_taux_bp(550), "5,50 %");
        assert_eq!(pourcentage(50_000, 100_000), 50);
    }

    #[test]
    fn totaux_en_franchise_sans_tva() {
        let lignes = vec![Ligne::new("Prestation", 2.0, 50_000, 2000)];
        let t = totaux(&lignes, RegimeTva::Franchise);
        assert_eq!(t.ht, 100_000);
        assert_eq!(t.tva, 0);
        assert_eq!(t.ttc, 100_000);
        assert_eq!(
            RegimeTva::Franchise.mention_facture(),
            Some("TVA non applicable, art. 293 B du CGI")
        );
    }

    #[test]
    fn totaux_tva_reelle_et_ventilation() {
        let lignes = vec![
            Ligne::new("Article", 1.0, 10_000, 2000), // 100 € HT à 20 %
            Ligne::new("Port", 1.0, 1_000, 2000),     // 10 € HT à 20 %
            Ligne::new("Livre", 1.0, 2_000, 550),     // 20 € HT à 5,5 %
        ];
        let t = totaux(&lignes, RegimeTva::Reel);
        assert_eq!(t.ht, 13_000);
        assert_eq!(t.tva, 2_310);
        assert_eq!(t.ttc, 15_310);
        let ventilation = tva_par_taux(&lignes, RegimeTva::Reel);
        assert_eq!(ventilation.len(), 2);
        assert_eq!(ventilation[0].taux_bp, 550);
        assert_eq!(ventilation[0].base_ht, 2_000);
        assert_eq!(ventilation[0].tva, 110);
        assert_eq!(ventilation[1].taux_bp, 2000);
        assert_eq!(ventilation[1].base_ht, 11_000);
        assert_eq!(ventilation[1].tva, 2_200);
    }

    #[test]
    fn remise_de_ligne() {
        let ligne = Ligne::new("Maintenance", 2.0, 50_000, 2000).avec_remise(1_000);
        assert_eq!(ligne.montant_remise(), 10_000);
        assert_eq!(ligne.total_ht(), 90_000);
        let t = totaux(&[ligne], RegimeTva::Reel);
        assert_eq!(t.ht, 90_000);
        assert_eq!(t.tva, 18_000);
        assert_eq!(t.remise, 10_000);
    }

    #[test]
    fn numerotation_et_immuabilite() {
        assert_eq!(numero_document(TypeDocument::Facture, 2026, 42), "FA-2026-0042");
        assert_eq!(numero_document(TypeDocument::Devis, 2026, 7), "DEV-2026-0007");
        assert!(peut_modifier(StatutDocument::Brouillon));
        assert!(!peut_modifier(StatutDocument::Valide));
        assert!(!peut_modifier(StatutDocument::Paye));
        // `depuis_code` est une fonction associée (pas une méthode) : le code
        // persistant est traduit sans instance préalable.
        assert_eq!(
            StatutDocument::depuis_code("partiel"),
            StatutDocument::PartiellementPaye
        );
        assert_eq!(
            StatutDocument::depuis_code("envoye"),
            StatutDocument::Envoye
        );
        assert_eq!(StatutDocument::depuis_code("inconnu"), StatutDocument::Brouillon);
    }

    #[test]
    fn paiements_partiels_et_penalites() {
        assert_eq!(reste_a_payer(10_000, 4_000), 6_000);
        assert_eq!(reste_a_payer(10_000, 12_000), 0);
        assert_eq!(statut_apres_paiement(10_000, 0), StatutDocument::Valide);
        assert_eq!(statut_apres_paiement(10_000, 5_000), StatutDocument::PartiellementPaye);
        assert_eq!(statut_apres_paiement(10_000, 10_000), StatutDocument::Paye);
        // 1 000 € TTC, 30 jours de retard, 10 % annuel → 8,22 €
        assert_eq!(penalites_retard(100_000, 30, 1_000), 822);
        assert_eq!(penalites_retard(100_000, 0, 1_000), 0);
    }

    #[test]
    fn document_retard_et_relances() {
        let mut doc = Document {
            numero: "FA-2026-0001".into(),
            statut: StatutDocument::Valide,
            date_doc: "2026-01-01".into(),
            echeance: "2026-01-31".into(),
            lignes: vec![Ligne::new("Prestation", 1.0, 100_000, 2000)],
            ..Document::default()
        };
        assert!(doc.en_retard("2026-03-15"));
        assert_eq!(doc.niveau_relance("2026-01-31"), 0);
        assert_eq!(doc.niveau_relance("2026-02-10"), 1);
        assert_eq!(doc.niveau_relance("2026-02-20"), 2);
        assert_eq!(doc.niveau_relance("2026-04-20"), 3);
        assert!(doc.penalites(RegimeTva::Franchise, "2026-08-09") > 0);
        doc.statut = StatutDocument::Paye;
        assert!(!doc.en_retard("2026-03-15"));
        assert_eq!(doc.niveau_relance("2026-03-15"), 0);
    }

    #[test]
    fn marge_vinted_et_dac7() {
        let mut a = Article::nouveau("Zara", "Veste");
        a.prix_achat = 800;
        a.composition = "100 % coton".into();
        let v = Vente::nouvelle(1, 2_500, 200);
        assert_eq!(marge_nette(&a, &v), 1_500); // 25 - 2 - 8 = 15 €
        assert_eq!(marge_pourcentage(&a, &v), 60);
        assert!(alerte_dac7(30, 0).is_some());
        assert!(alerte_dac7(0, 200_000).is_some());
        assert!(alerte_dac7(3, 10_000).is_none());
    }

    #[test]
    fn seuils_micro() {
        let e = etat_seuils(10_000 * 100, 5_000 * 100, SEUILS_2026);
        assert!(e.alertes.is_empty());
        assert_eq!(e.pct_services(), 5);
        let e2 = etat_seuils(0, 90_000 * 100, SEUILS_2026);
        assert!(!e2.alertes.is_empty());
    }

    #[test]
    fn dates_sans_dependance() {
        assert_eq!(date_depuis_jours(0), "1970-01-01");
        assert_eq!(date_depuis_jours(20_000), "2024-10-04");
        assert_eq!(jours_depuis_date("2024-10-04"), Some(20_000));
        assert_eq!(annee_de("2026-10-30"), 2026);
        assert_eq!(mois_de("2026-10-30"), 10);
        assert_eq!(periode_de("2026-10-30"), "2026-10");
        assert_eq!(date_fr("2026-10-30"), "30/10/2026");
        assert_eq!(ajouter_jours("2026-01-31", 30), "2026-03-02");
        assert_eq!(jours_entre("2026-01-01", "2026-02-01"), 31);
        assert!(date_valide("2026-02-29") == false);
        assert!(date_valide("2024-02-29"));
        assert_eq!(jours_du_mois(2026, 2), 28);
        assert_eq!(jours_du_mois(2024, 2), 29);
        assert_eq!(jour_semaine(jours_depuis_date("2026-11-02").unwrap()), 0); // lundi
    }

    #[test]
    fn horodatage_paris() {
        let h = horodatage();
        assert_eq!(h.len(), 19, "{h}");
        assert!(date_valide(&h[..10]), "{h}");
    }

    #[test]
    fn conversions_devises() {
        // 1 USD = 0,92 EUR → 100,00 USD = 92,00 EUR
        assert_eq!(convertir(10_000, 920_000), 9_200);
        assert_eq!(convertir(10_000, 1_000_000), 10_000);
        assert_eq!(parse_taux("0,92").unwrap(), 9_200);
    }

    #[test]
    fn controle_des_mentions_obligatoires() {
        let societe = Societe {
            nom: "Dupont Conseil".into(),
            siren: "123456789".into(),
            adresse: "1 rue de la Paix".into(),
            ville: "Nice".into(),
            ..Societe::default()
        };
        let client = Client::nouveau("Atelier Dupont", TypeClient::Professionnel);
        let doc = Document {
            numero: "FA-2026-0001".into(),
            date_doc: "2026-10-30".into(),
            date_prestation: "2026-10-30".into(),
            echeance: "2026-11-29".into(),
            lignes: vec![Ligne::new("Maintenance", 2.0, 50_000, 2000)],
            ..Document::default()
        };
        let controle = controle_mentions(&societe, &client, &doc);
        assert!(controle.conforme(), "{:?}", controle.manquants);

        let incomplete = Societe::default();
        let controle = controle_mentions(&incomplete, &client, &doc);
        assert!(!controle.conforme());
        assert!(controle.manquants.iter().any(|m| m.contains("SIREN")));
    }

    #[test]
    fn etiquettes_et_recherche() {
        let etiquettes = etiquettes_depuis_texte("maintenance, urgent ,");
        assert_eq!(etiquettes, vec!["maintenance", "urgent"]);
        assert_eq!(etiquettes_vers_texte(&etiquettes), "maintenance,urgent");
        assert!(correspond("Atelier Dupont", "dupont atelier"));
        assert!(correspond("Atelier Dupont", ""));
        assert!(!correspond("Atelier Dupont", "martin"));
    }

    #[test]
    fn fiche_annonce_vinted() {
        let mut a = Article::nouveau("Zara", "Veste en jean");
        a.taille = "M".into();
        a.composition = "100 % coton".into();
        a.prix_achat = 800;
        let texte = a.annonce();
        assert!(texte.contains("Marque : Zara"));
        assert!(texte.contains("Composition : 100 % coton"));
        assert_eq!(a.prix_vente_conseille(), 2_000);
    }
}
