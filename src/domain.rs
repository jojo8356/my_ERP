//! Règles métier pures de l'ERP (aucune entrée/sortie, aucune base de données).
//! Tout est testable unitairement : calculs d'argent en centimes entiers,
//! TVA (franchise en base ou TVA réelle), numérotation, marges Vinted, seuils micro.

use std::fmt;

/// Montant en centimes (jamais de flottant pour l'argent — CDC §5.3).
pub type Cents = i64;

pub fn format_eur(cents: Cents) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{}{},{:02} €", sign, group_thousands(abs / 100), abs % 100)
}

fn group_thousands(mut n: u64) -> String {
    let mut parts = Vec::new();
    loop {
        parts.push(format!("{:03}", n % 1000));
        n /= 1000;
        if n == 0 {
            break;
        }
    }
    parts.reverse();
    let mut first = parts.remove(0);
    if first.starts_with('0') && first.len() > 1 && parts.len() > 0 || first.len() == 3 && parts.is_empty() {
        // garder tel quel
    }
    while first.len() > 1 && first.starts_with('0') {
        first.remove(0);
    }
    if parts.is_empty() {
        first
    } else {
        format!("{} {}", first, parts.join(" "))
    }
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
    pub fn mention_facture(&self) -> Option<&'static str> {
        match self {
            Self::Franchise => Some("TVA non applicable, art. 293 B du CGI"),
            Self::Reel => None,
        }
    }
}

/// Une ligne de devis/facture. `tva_bp` = taux en points de base (20 % = 2000).
#[derive(Debug, Clone, PartialEq)]
pub struct Ligne {
    pub designation: String,
    pub quantite_milli: i64, // quantité × 1000 (2,5 h = 2500)
    pub prix_unitaire_ht: Cents,
    pub tva_bp: i32,
}

impl Ligne {
    pub fn new(designation: &str, quantite: f64, prix_unitaire_ht: Cents, tva_bp: i32) -> Self {
        Self {
            designation: designation.to_string(),
            quantite_milli: (quantite * 1000.0).round() as i64,
            prix_unitaire_ht,
            tva_bp,
        }
    }
    pub fn total_ht(&self) -> Cents {
        (self.quantite_milli * self.prix_unitaire_ht + 500) / 1000
    }
    pub fn total_tva(&self, regime: RegimeTva) -> Cents {
        match regime {
            RegimeTva::Franchise => 0,
            RegimeTva::Reel => (self.total_ht() * i64::from(self.tva_bp) + 5000) / 10_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Totaux {
    pub ht: Cents,
    pub tva: Cents,
    pub ttc: Cents,
}

pub fn totaux(lignes: &[Ligne], regime: RegimeTva) -> Totaux {
    let ht: Cents = lignes.iter().map(|l| l.total_ht()).sum();
    let tva: Cents = lignes.iter().map(|l| l.total_tva(regime)).sum();
    Totaux {
        ht,
        tva,
        ttc: ht + tva,
    }
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatutDocument {
    Brouillon,
    Envoye,
    Accepte,
    Refuse,
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
            Self::Valide => "validée",
            Self::Paye => "payée",
            Self::PartiellementPaye => "partiellement payée",
            Self::Annule => "annulée",
        }
    }
    pub fn symbole(&self) -> &'static str {
        match self {
            Self::Paye | Self::Accepte => "✓",
            Self::Brouillon => "…",
            Self::Refuse | Self::Annule => "✗",
            _ => "!",
        }
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

// ---------------------------------------------------------------- Clients

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeClient {
    Particulier,
    Professionnel,
}

impl TypeClient {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "particulier" | "part" => Ok(Self::Particulier),
            "pro" | "professionnel" => Ok(Self::Professionnel),
            _ => Err(format!("type de client inconnu : « {s} » (pro|particulier)")),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Particulier => "particulier",
            Self::Professionnel => "professionnel",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Client {
    pub id: i64,
    pub nom: String,
    pub type_client: TypeClient,
    pub email: String,
    pub telephone: String,
    pub adresse: String,
    pub siren: String,
    pub tva_intracom: String,
    pub notes: String,
}

impl fmt::Display for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.nom, self.type_client.as_str())
    }
}

// ---------------------------------------------------------------- Vinted / vêtements

#[derive(Debug, Clone, PartialEq)]
pub struct Article {
    pub id: i64,
    pub marque: String,
    pub description: String,
    pub taille: String,
    pub composition: String, // obligatoire UE 1007/2011 (CDC §11.3)
    pub etat: String,
    pub prix_achat: Cents,
    pub date_achat: String, // AAAA-MM-JJ
    pub statut: String,     // en_stock | en_ligne | vendu
}

#[derive(Debug, Clone, PartialEq)]
pub struct Vente {
    pub id: i64,
    pub article_id: i64,
    pub date_vente: String,
    pub prix_vente: Cents,
    pub frais: Cents, // plateforme + port à charge du vendeur
    pub plateforme: String,
}

/// Marge nette d'une vente d'achat-revente (CDC EF-VIN-02).
pub fn marge_nette(article: &Article, vente: &Vente) -> Cents {
    vente.prix_vente - vente.frais - article.prix_achat
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
}

pub const SEUILS_2026: SeuilsMicro = SeuilsMicro {
    ca_global_max: 203_100 * 100,
    ca_services_max: 83_600 * 100,
    franchise_tva_services: 37_500 * 100,
    franchise_tva_ventes: 85_000 * 100,
};

#[derive(Debug, Clone, PartialEq)]
pub struct EtatSeuils {
    pub ca_ventes: Cents,
    pub ca_services: Cents,
    pub alertes: Vec<String>,
}

pub fn etat_seuils(ca_ventes: Cents, ca_services: Cents, seuils: SeuilsMicro) -> EtatSeuils {
    let mut alertes = Vec::new();
    let global = ca_ventes + ca_services;
    if global > seuils.ca_global_max {
        alertes.push(format!(
            "CA global {} au-delà du plafond micro ({} ) : sortie du régime micro",
            format_eur(global),
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

// ---------------------------------------------------------------- Divers

/// Date du jour au format AAAA-MM-JJ, calculée sans dépendance externe.
pub fn aujourdhui() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    date_depuis_jours((secs / 86_400) as i64)
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

pub fn annee_de(date: &str) -> u32 {
    date.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(2026)
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
            _ => c,
        })
        .collect()
}

/// Recherche floue simple : toutes les parties cherchées apparaissent (insensible casse/accents).
pub fn correspond(haystack: &str, needle: &str) -> bool {
    let h = slug_recherche(haystack);
    slug_recherche(needle)
        .split_whitespace()
        .all(|part| h.contains(part))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_et_parse_eur() {
        assert_eq!(format_eur(0), "0,00 €");
        assert_eq!(format_eur(4_250), "42,50 €");
        assert_eq!(format_eur(123_456), "1 234,56 €");
        assert_eq!(format_eur(-500), "-5,00 €");
        assert_eq!(parse_eur("42,50").unwrap(), 4_250);
        assert_eq!(parse_eur("1 234,56 €").unwrap(), 123_456);
        assert_eq!(parse_eur("100").unwrap(), 10_000);
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
    fn totaux_tva_reelle() {
        let lignes = vec![
            Ligne::new("Article", 1.0, 10_000, 2000),  // 100 € HT à 20 %
            Ligne::new("Port", 1.0, 1_000, 2000),      // 10 € HT à 20 %
        ];
        let t = totaux(&lignes, RegimeTva::Reel);
        assert_eq!(t.ht, 11_000);
        assert_eq!(t.tva, 2_200);
        assert_eq!(t.ttc, 13_200);
    }

    #[test]
    fn numerotation_et_immuabilite() {
        assert_eq!(numero_document(TypeDocument::Facture, 2026, 42), "FA-2026-0042");
        assert_eq!(numero_document(TypeDocument::Devis, 2026, 7), "DEV-2026-0007");
        assert!(peut_modifier(StatutDocument::Brouillon));
        assert!(!peut_modifier(StatutDocument::Valide));
        assert!(!peut_modifier(StatutDocument::Paye));
    }

    #[test]
    fn paiements_partiels() {
        assert_eq!(reste_a_payer(10_000, 4_000), 6_000);
        assert_eq!(reste_a_payer(10_000, 12_000), 0);
        assert_eq!(statut_apres_paiement(10_000, 0), StatutDocument::Valide);
        assert_eq!(statut_apres_paiement(10_000, 5_000), StatutDocument::PartiellementPaye);
        assert_eq!(statut_apres_paiement(10_000, 10_000), StatutDocument::Paye);
    }

    #[test]
    fn marge_vinted_et_dac7() {
        let a = Article {
            id: 1,
            marque: "Zara".into(),
            description: "Veste".into(),
            taille: "M".into(),
            composition: "100 % coton".into(),
            etat: "très bon".into(),
            prix_achat: 800,
            date_achat: "2026-10-01".into(),
            statut: "vendu".into(),
        };
        let v = Vente {
            id: 1,
            article_id: 1,
            date_vente: "2026-10-20".into(),
            prix_vente: 2_500,
            frais: 200,
            plateforme: "Vinted".into(),
        };
        assert_eq!(marge_nette(&a, &v), 1_500); // 25 - 2 - 8 = 15 €
        assert!(alerte_dac7(30, 0).is_some());
        assert!(alerte_dac7(0, 200_000).is_some());
        assert!(alerte_dac7(3, 10_000).is_none());
    }

    #[test]
    fn seuils_micro() {
        let e = etat_seuils(10_000 * 100, 5_000 * 100, SEUILS_2026);
        assert!(e.alertes.is_empty());
        let e2 = etat_seuils(0, 90_000 * 100, SEUILS_2026);
        assert!(!e2.alertes.is_empty());
    }

    #[test]
    fn dates_sans_dependance() {
        assert_eq!(date_depuis_jours(0), "1970-01-01");
        assert_eq!(date_depuis_jours(20_000), "2024-10-04");
        assert_eq!(annee_de("2026-10-30"), 2026);
    }
}
