//! Interface TUI Ratatui + Crossterm (ADR-005) — plein écran dans le
//! terminal, toujours aucune GUI (CDC §6). Chromat reste la référence
//! couleurs du projet : la TUI applique la palette RGB définie dans
//! `theme.rs` aux styles Ratatui ; aucun fond coloré, écriture seule.
//!
//! Lancement : `my-erp tui`
//! Touches : Tab module · ↑↓ naviguer · f formulaires · n nouveau (selon le
//! module) · d devis · p payer · v valider · Entrée vendre (Vinted) · ? aide
//! Dans un formulaire : Tab/↑↓ champ suivant/précédent · ←→ choix d'une
//! liste · Entrée champ suivant (ou enregistrer sur le dernier champ) ·
//! Ctrl+S enregistrer · Échap annuler.

use crate::domain::*;
use crate::store::*;
use crate::theme;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, ExecutableCommand};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};

pub const MODULES: [&str; 5] = ["Tableau de bord", "Clients", "Factures", "Vinted", "Comptabilité"];

/// Libellés du menu des formulaires, ouvert depuis n'importe où par `f`.
pub const CHOIX_FORMULAIRES: [&str; 7] = [
    "Nouveau client",
    "Nouvelle facture",
    "Nouveau devis",
    "Nouvel article Vinted",
    "Vendre un article Vinted…",
    "Régime TVA de la société",
    "Exporter le FEC…",
];

#[derive(Debug, Clone, PartialEq)]
pub struct VueTui {
    pub module: usize,
    pub selection: usize,
    pub aide: bool,
    pub menu_formulaires: bool,
    pub choix_menu: usize,
    pub formulaire: Option<Formulaire>,
    pub message: String,
}

impl Default for VueTui {
    fn default() -> Self {
        Self {
            module: 0,
            selection: 0,
            aide: false,
            menu_formulaires: false,
            choix_menu: 0,
            formulaire: None,
            message: "Tab = module · f = formulaires · n = nouveau · ? = aide · q = quitter".to_string(),
        }
    }
}

/// Données affichables d'un module. Les collections servent à relier la
/// ligne sélectionnée à l'objet métier concerné par les actions/formulaires.
pub struct ContenuModule {
    pub lignes: Vec<String>,
    pub factures: Vec<Document>,
    pub clients: Vec<Client>,
    pub articles: Vec<Article>,
}

impl ContenuModule {
    fn vide(lignes: Vec<String>) -> Self {
        Self { lignes, factures: Vec::new(), clients: Vec::new(), articles: Vec::new() }
    }
}

pub fn charger_contenu(store: &mut dyn Store, module: usize) -> Result<ContenuModule, String> {
    let regime = store.societe()?.regime;
    match module {
        0 => {
            let annee = annee_de(&aujourdhui());
            let (ventes, services) = store.ca_annuel(annee)?;
            let etat = etat_seuils(ventes, services, SEUILS_2026);
            let mut lignes = vec![
                format!("CA ventes {annee}   : {}", format_eur(ventes)),
                format!("CA services {annee} : {}", format_eur(services)),
                format!("CA global          : {}", format_eur(ventes + services)),
            ];
            for a in &etat.alertes {
                lignes.push(format!("⚠ {a}"));
            }
            let impayes = store
                .documents(Some(TypeDocument::Facture))?
                .into_iter()
                .filter(|d| matches!(d.statut, StatutDocument::Valide | StatutDocument::PartiellementPaye))
                .count();
            lignes.push(format!("Factures en attente de paiement : {impayes}"));
            lignes.push(String::new());
            lignes.push("Formulaires : f (tous) · n (régime TVA société depuis ce module)".to_string());
            Ok(ContenuModule::vide(lignes))
        }
        1 => {
            let clients = store.clients()?;
            let lignes = clients
                .iter()
                .map(|c| format!("{:>4}  {:<30} {:<14} {}", c.id, c.nom, c.type_client.as_str(), c.email))
                .collect();
            Ok(ContenuModule { lignes, factures: Vec::new(), clients, articles: Vec::new() })
        }
        2 => {
            let factures = store.documents(Some(TypeDocument::Facture))?;
            let lignes = factures
                .iter()
                .map(|d| {
                    let t = d.totaux(regime);
                    format!(
                        "{} {:<14} {}  TTC {:>12}  payé {:>12}  ({})",
                        d.statut.symbole(),
                        d.numero,
                        d.date_doc,
                        format_eur(t.ttc),
                        format_eur(d.paye),
                        d.statut.as_str()
                    )
                })
                .collect();
            Ok(ContenuModule { lignes, factures, clients: Vec::new(), articles: Vec::new() })
        }
        3 => {
            let articles = store.articles()?;
            let mut lignes: Vec<String> = articles
                .iter()
                .map(|a| format!("{:>4}  {:<14} {:<26} taille {:<4} achat {:>10}  [{}]", a.id, a.marque, a.description, a.taille, format_eur(a.prix_achat), a.statut))
                .collect();
            let ventes = store.ventes()?;
            let marge: Cents = ventes
                .iter()
                .map(|v| store.article_par_id(v.article_id).map(|a| marge_nette(&a, v)).unwrap_or(0))
                .sum();
            lignes.push(String::new());
            lignes.push(format!("Ventes : {} · marge nette cumulée : {}", ventes.len(), format_eur(marge)));
            Ok(ContenuModule { lignes, factures: Vec::new(), clients: Vec::new(), articles })
        }
        _ => {
            let annee = annee_de(&aujourdhui());
            let (ventes, services) = store.ca_annuel(annee)?;
            let e = etat_seuils(ventes, services, SEUILS_2026);
            let mut lignes = vec![
                format!("Micro-entreprise — activité mixte ({annee})"),
                format!("  CA ventes   : {}  (franchise TVA dès {})", format_eur(e.ca_ventes), format_eur(SEUILS_2026.franchise_tva_ventes)),
                format!("  CA services : {}  (plafond services {})", format_eur(e.ca_services), format_eur(SEUILS_2026.ca_services_max)),
                format!("  CA global   : {}  (plafond global {})", format_eur(e.ca_ventes + e.ca_services), format_eur(SEUILS_2026.ca_global_max)),
            ];
            if e.alertes.is_empty() {
                lignes.push("  ✓ Aucune alerte de seuil".to_string());
            }
            for a in &e.alertes {
                lignes.push(format!("  ⚠ {a}"));
            }
            lignes.push(String::new());
            lignes.push("Formulaires : n ou x = export FEC · s = régime TVA société".to_string());
            Ok(ContenuModule::vide(lignes))
        }
    }
}

// ---------------------------------------------------------------- Formulaires

#[derive(Debug, Clone, PartialEq)]
pub enum TypeFormulaire {
    Client,
    Document(TypeDocument),
    Paiement,
    Article,
    Vente,
    Societe,
    ExportFec,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Champ {
    pub nom: &'static str,
    pub valeur: String,
    /// Non vide pour un champ à choix (←/→ pour changer, saisie ignorée).
    pub options: &'static [&'static str],
}

impl Champ {
    fn texte(nom: &'static str, defaut: impl Into<String>) -> Self {
        Self { nom, valeur: defaut.into(), options: &[] }
    }
    fn choix(nom: &'static str, options: &'static [&'static str], defaut: &str) -> Self {
        Self { nom, valeur: defaut.to_string(), options }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Formulaire {
    pub type_form: TypeFormulaire,
    pub titre: String,
    pub champs: Vec<Champ>,
    pub actif: usize,
    pub erreur: Option<String>,
    /// Contexte invisible : facture à payer / article à vendre.
    pub numero_document: String,
    pub article_id: i64,
}

impl Formulaire {
    pub fn client() -> Self {
        Self {
            type_form: TypeFormulaire::Client,
            titre: "Nouveau client".to_string(),
            champs: vec![
                Champ::texte("Nom *", ""),
                Champ::choix("Type", &["pro", "particulier"], "pro"),
                Champ::texte("E-mail", ""),
                Champ::texte("Téléphone", ""),
                Champ::texte("Adresse", ""),
                Champ::texte("SIREN", ""),
                Champ::texte("TVA intracommunautaire", ""),
                Champ::texte("Notes", ""),
            ],
            actif: 0,
            erreur: None,
            numero_document: String::new(),
            article_id: 0,
        }
    }

    pub fn document(type_doc: TypeDocument) -> Self {
        let titre = if type_doc == TypeDocument::Devis { "Nouveau devis" } else { "Nouvelle facture" };
        Self {
            type_form: TypeFormulaire::Document(type_doc),
            titre: titre.to_string(),
            champs: vec![
                Champ::texte("Client (nom ou e-mail) *", ""),
                Champ::choix("Activité", &["services", "ventes"], "services"),
                Champ::texte("Date", aujourdhui()),
                Champ::texte("Échéance (AAAA-MM-JJ)", ""),
                Champ::texte("Désignation *", ""),
                Champ::texte("Quantité", "1"),
                Champ::texte("Prix unitaire HT (€)", ""),
                Champ::texte("TVA (points de base, 2000 = 20 %)", "2000"),
            ],
            actif: 0,
            erreur: None,
            numero_document: String::new(),
            article_id: 0,
        }
    }

    pub fn paiement(document: &Document, reste: Cents) -> Self {
        Self {
            type_form: TypeFormulaire::Paiement,
            titre: format!("Payer {}", document.numero),
            champs: vec![
                Champ::texte("Montant (€)", format_eur(reste)),
                Champ::texte("Date", aujourdhui()),
                Champ::choix("Moyen", &["virement", "carte", "espèces", "chèque", "autre"], "virement"),
            ],
            actif: 0,
            erreur: None,
            numero_document: document.numero.clone(),
            article_id: 0,
        }
    }

    pub fn article() -> Self {
        Self {
            type_form: TypeFormulaire::Article,
            titre: "Nouvel article Vinted".to_string(),
            champs: vec![
                Champ::texte("Marque *", ""),
                Champ::texte("Description *", ""),
                Champ::texte("Taille", ""),
                Champ::texte("Composition textile *", ""),
                Champ::choix("État", &["très bon", "bon", "correct", "à réparer"], "très bon"),
                Champ::texte("Prix d'achat (€)", ""),
                Champ::texte("Date d'achat", aujourdhui()),
            ],
            actif: 0,
            erreur: None,
            numero_document: String::new(),
            article_id: 0,
        }
    }

    pub fn vente(article: &Article) -> Self {
        Self {
            type_form: TypeFormulaire::Vente,
            titre: format!("Vendre : {} {}", article.marque, article.description),
            champs: vec![
                Champ::texte("Prix de vente (€) *", ""),
                Champ::texte("Frais plateforme/port (€)", "0"),
                Champ::texte("Date de vente", aujourdhui()),
                Champ::texte("Plateforme", "Vinted"),
            ],
            actif: 0,
            erreur: None,
            numero_document: String::new(),
            article_id: article.id,
        }
    }

    pub fn societe(regime: RegimeTva) -> Self {
        Self {
            type_form: TypeFormulaire::Societe,
            titre: "Société — régime de TVA".to_string(),
            champs: vec![Champ::choix("Régime TVA", &["franchise", "reel"], regime.as_str())],
            actif: 0,
            erreur: None,
            numero_document: String::new(),
            article_id: 0,
        }
    }

    pub fn export_fec() -> Self {
        Self {
            type_form: TypeFormulaire::ExportFec,
            titre: "Export FEC (expert-comptable)".to_string(),
            champs: vec![Champ::texte("Fichier de sortie", "FEC.txt")],
            actif: 0,
            erreur: None,
            numero_document: String::new(),
            article_id: 0,
        }
    }

    pub fn suivant(&mut self) {
        if self.actif + 1 < self.champs.len() {
            self.actif += 1;
        }
    }

    pub fn precedent(&mut self) {
        self.actif = self.actif.saturating_sub(1);
    }

    pub fn est_dernier_champ(&self) -> bool {
        self.actif + 1 >= self.champs.len()
    }

    pub fn saisir(&mut self, c: char) {
        let champ = &mut self.champs[self.actif];
        if champ.options.is_empty() {
            champ.valeur.push(c);
        }
    }

    pub fn effacer(&mut self) {
        let champ = &mut self.champs[self.actif];
        if champ.options.is_empty() {
            champ.valeur.pop();
        }
    }

    /// ←/→ sur un champ à choix ; sans effet sur un champ texte.
    pub fn defiler_choix(&mut self, sens: i32) {
        let champ = &mut self.champs[self.actif];
        if champ.options.is_empty() {
            return;
        }
        let pos = champ.options.iter().position(|o| *o == champ.valeur).unwrap_or(0) as i32;
        let n = champ.options.len() as i32;
        let suivant = (pos + sens).rem_euclid(n) as usize;
        champ.valeur = champ.options[suivant].to_string();
    }

    fn v(&self, i: usize) -> String {
        self.champs.get(i).map(|c| c.valeur.trim().to_string()).unwrap_or_default()
    }
}

fn valider_date(s: &str, champ: &str) -> Result<(), String> {
    let ok = s.len() == 10
        && s.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() });
    if ok {
        Ok(())
    } else {
        Err(format!("{champ} : date au format AAAA-MM-JJ attendue, reçu « {s} »"))
    }
}

/// Valide le formulaire et applique son effet au `Store`.
/// Retourne le message de succès à afficher ; en cas d'erreur, le
/// formulaire reste ouvert et affiche le motif (aucune écriture partielle).
pub fn soumettre_formulaire(store: &mut dyn Store, form: &Formulaire) -> Result<String, String> {
    match &form.type_form {
        TypeFormulaire::Client => {
            let nom = form.v(0);
            if nom.is_empty() {
                return Err("le nom du client est obligatoire".into());
            }
            let c = store.creer_client(Client {
                id: 0,
                nom: nom.clone(),
                type_client: TypeClient::from_str(&form.v(1))?,
                email: form.v(2),
                telephone: form.v(3),
                adresse: form.v(4),
                siren: form.v(5),
                tva_intracom: form.v(6),
                notes: form.v(7),
            })?;
            let _ = store.journaliser("tui.formulaire.client", &c.nom);
            Ok(format!("✓ Client n°{} créé : {}", c.id, c.nom))
        }
        TypeFormulaire::Document(type_doc) => {
            let recherche = form.v(0);
            if recherche.is_empty() {
                return Err("indiquez le client (nom ou e-mail)".into());
            }
            let client = store
                .chercher_clients(&recherche)?
                .into_iter()
                .next()
                .ok_or_else(|| format!("client « {recherche} » introuvable — créez-le d'abord (module Clients, touche n)"))?;
            let designation = form.v(4);
            if designation.is_empty() {
                return Err("la désignation de la ligne est obligatoire".into());
            }
            let quantite: f64 = form.v(5).replace(',', ".").parse().map_err(|_| "quantité illisible".to_string())?;
            if quantite <= 0.0 {
                return Err("la quantité doit être positive".into());
            }
            let prix = parse_eur(&form.v(6))?;
            let tva_bp: i32 = form.v(7).parse().map_err(|_| "taux TVA illisible (points de base)".to_string())?;
            let date = form.v(2);
            valider_date(&date, "Date")?;
            let echeance = form.v(3);
            if !echeance.is_empty() {
                valider_date(&echeance, "Échéance")?;
            }
            let doc = store.creer_document(
                *type_doc,
                client.id,
                &date,
                &echeance,
                &form.v(1),
                vec![Ligne::new(&designation, quantite, prix, tva_bp)],
            )?;
            let _ = store.journaliser("tui.formulaire.document", &doc.numero);
            Ok(format!("✓ {} créé : {} (brouillon){}", type_doc.as_str(), doc.numero, if *type_doc == TypeDocument::Facture { " — v pour valider" } else { "" }))
        }
        TypeFormulaire::Paiement => {
            let montant = parse_eur(&form.v(0))?;
            if montant <= 0 {
                return Err("le montant du paiement doit être positif".into());
            }
            let date = form.v(1);
            valider_date(&date, "Date")?;
            let doc = store.ajouter_paiement(&form.numero_document, montant, &date, &form.v(2))?;
            let regime = store.societe()?.regime;
            let _ = store.journaliser("tui.formulaire.paiement", &doc.numero);
            Ok(format!(
                "✓ Paiement enregistré sur {} — statut : {} (reste {})",
                doc.numero,
                doc.statut.as_str(),
                format_eur(reste_a_payer(doc.totaux(regime).ttc, doc.paye))
            ))
        }
        TypeFormulaire::Article => {
            if form.v(0).is_empty() || form.v(1).is_empty() {
                return Err("marque et description sont obligatoires".into());
            }
            if form.v(3).is_empty() {
                return Err("composition textile obligatoire (règlement UE 1007/2011)".into());
            }
            let prix_achat = parse_eur(&form.v(5))?;
            let date = form.v(6);
            valider_date(&date, "Date d'achat")?;
            let a = store.creer_article(Article {
                id: 0,
                marque: form.v(0),
                description: form.v(1),
                taille: form.v(2),
                composition: form.v(3),
                etat: form.v(4),
                prix_achat,
                date_achat: date,
                statut: "en_stock".to_string(),
            })?;
            let _ = store.journaliser("tui.formulaire.article", &a.marque);
            Ok(format!("✓ Article n°{} ajouté : {} {}", a.id, a.marque, a.description))
        }
        TypeFormulaire::Vente => {
            let article = store.article_par_id(form.article_id)?;
            if article.statut == "vendu" {
                return Err("cet article est déjà vendu".into());
            }
            let prix_vente = parse_eur(&form.v(0))?;
            if prix_vente <= 0 {
                return Err("le prix de vente doit être positif".into());
            }
            let frais = parse_eur(&form.v(1))?;
            let date = form.v(2);
            valider_date(&date, "Date de vente")?;
            let vente = store.enregistrer_vente(Vente {
                id: 0,
                article_id: form.article_id,
                date_vente: date,
                prix_vente,
                frais,
                plateforme: form.v(3),
            })?;
            let _ = store.journaliser("tui.formulaire.vente", &vente.id.to_string());
            Ok(format!(
                "✓ Vente enregistrée : {} {} vendu {} — marge nette {}",
                article.marque,
                article.description,
                format_eur(prix_vente),
                format_eur(marge_nette(&article, &vente))
            ))
        }
        TypeFormulaire::Societe => {
            let regime = RegimeTva::from_str(&form.v(0))?;
            store.set_regime_tva(regime)?;
            let _ = store.journaliser("tui.formulaire.societe", regime.as_str());
            Ok(format!("✓ Régime TVA : {}", regime.as_str()))
        }
        TypeFormulaire::ExportFec => {
            let fichier = form.v(0);
            if fichier.is_empty() {
                return Err("indiquez le fichier de sortie".into());
            }
            let lignes = generer_fec(store)?;
            std::fs::write(&fichier, lignes.join("\n") + "\n").map_err(|e| format!("écriture du FEC : {e}"))?;
            let _ = store.journaliser("tui.formulaire.fec", &fichier);
            Ok(format!("✓ FEC écrit dans {fichier} ({} lignes) — à faire valider par l'expert-comptable", lignes.len()))
        }
    }
}

// ---------------------------------------------------------------- Styles Ratatui

fn style_rgb((r, g, b): (u8, u8, u8)) -> Style {
    Style::default().fg(Color::Rgb(r, g, b))
}

fn style_blanc() -> Style {
    style_rgb(theme::BLANC_RGB)
}

fn style_titre() -> Style {
    style_blanc().add_modifier(Modifier::BOLD)
}

fn style_violet() -> Style {
    style_rgb(theme::VIOLET_RGB)
}

fn style_selection() -> Style {
    style_violet().add_modifier(Modifier::BOLD)
}

fn style_message(message: &str) -> Style {
    if message.starts_with('✓') {
        style_rgb(theme::VERT_RGB)
    } else if message.starts_with('✗') {
        style_rgb(theme::ROUGE_RGB).add_modifier(Modifier::BOLD)
    } else if message.contains('⚠') {
        style_rgb(theme::ORANGE_RGB)
    } else {
        style_blanc()
    }
}

// ---------------------------------------------------------------- Rendu Ratatui

fn zone_centree(pourcent_x: u16, pourcent_y: u16, zone: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pourcent_y) / 2),
            Constraint::Percentage(pourcent_y),
            Constraint::Percentage((100 - pourcent_y) / 2),
        ])
        .split(zone);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pourcent_x) / 2),
            Constraint::Percentage(pourcent_x),
            Constraint::Percentage((100 - pourcent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn dessiner(frame: &mut Frame, vue: &VueTui, contenu: &ContenuModule, societe_nom: &str) {
    let zones = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(frame.area());

    // Titre
    frame.render_widget(
        Paragraph::new(format!(" ERP — {societe_nom}")).style(style_titre()).block(Block::default().borders(Borders::ALL).border_style(style_blanc())),
        zones[0],
    );

    // Onglets des modules (cadre blanc, écriture violette, actif violet gras)
    let titres: Vec<Line> = MODULES.iter().map(|m| Line::from(*m)).collect();
    frame.render_widget(
        Tabs::new(titres)
            .select(vue.module)
            .style(style_violet())
            .highlight_style(style_selection())
            .divider("│")
            .block(Block::default().borders(Borders::ALL).border_style(style_blanc())),
        zones[1],
    );

    // Contenu du module
    let bloc = Block::default()
        .title(format!(" {} ", MODULES[vue.module]))
        .borders(Borders::ALL)
        .border_style(style_blanc())
        .title_style(style_titre());
    if contenu.lignes.is_empty() {
        frame.render_widget(
            Paragraph::new("(aucun élément — touche f pour ouvrir un formulaire, n pour créer)").block(bloc),
            zones[2],
        );
    } else {
        let items: Vec<ListItem> = contenu
            .lignes
            .iter()
            .map(|l| {
                let style = if l.contains('⚠') {
                    style_rgb(theme::ORANGE_RGB)
                } else if l.contains('✓') {
                    style_rgb(theme::VERT_RGB)
                } else {
                    Style::default()
                };
                ListItem::new(l.clone()).style(style)
            })
            .collect();
        let mut etat = ListState::default();
        if vue.module != 0 && vue.module != 4 {
            etat.select(Some(vue.selection.min(contenu.lignes.len() - 1)));
        }
        frame.render_stateful_widget(
            List::new(items).block(bloc).highlight_style(style_selection()).highlight_symbol("▸ "),
            zones[2],
            &mut etat,
        );
    }

    // Pied : message + aide-mémoire des touches
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(vue.message.clone(), style_message(&vue.message)),
            Line::styled("f formulaires · n nouveau · ↑↓ naviguer · Tab module · ? aide · q quitter", style_violet()),
        ])
        .wrap(Wrap { trim: true }),
        zones[3],
    );

    if vue.aide {
        dessiner_aide(frame);
    }
    if vue.menu_formulaires {
        dessiner_menu_formulaires(frame, vue.choix_menu);
    }
    if let Some(form) = &vue.formulaire {
        dessiner_formulaire(frame, form);
    }
}

fn dessiner_aide(frame: &mut Frame) {
    let zone = zone_centree(72, 70, frame.area());
    frame.render_widget(Clear, zone);
    let texte = vec![
        Line::styled("AIDE — touches de la TUI", style_titre()),
        Line::raw(""),
        Line::raw("  Tab / Maj+Tab     module suivant / précédent"),
        Line::raw("  ↑ / ↓  ou  j / k  monter / descendre dans la liste"),
        Line::raw("  f                 ouvrir le menu des formulaires"),
        Line::raw("  n                 nouveau… selon le module (client, facture, article, FEC, société)"),
        Line::raw("  d                 nouveau devis (module Factures)"),
        Line::raw("  v                 valider la facture sélectionnée"),
        Line::raw("  p                 payer la facture sélectionnée (formulaire)"),
        Line::raw("  Entrée            vendre l'article sélectionné (module Vinted)"),
        Line::raw("  s / x             société (régime TVA) / export FEC (module Comptabilité)"),
        Line::raw("  r                 rafraîchir les données"),
        Line::raw("  ?                 afficher / masquer cette aide"),
        Line::raw("  q                 quitter la TUI (le CLI reste disponible)"),
        Line::raw(""),
        Line::styled("Dans un formulaire", style_titre()),
        Line::raw("  Tab / Entrée      champ suivant · Maj+Tab champ précédent"),
        Line::raw("  ← / →             changer un champ à choix (type, activité, moyen…)"),
        Line::raw("  Ctrl+S            enregistrer · Échap annuler"),
        Line::raw(""),
        Line::styled("  Échap ou ? pour fermer", style_violet()),
    ];
    frame.render_widget(
        Paragraph::new(texte).block(Block::default().title(" Aide ").borders(Borders::ALL).border_style(style_blanc())),
        zone,
    );
}

fn dessiner_menu_formulaires(frame: &mut Frame, choix: usize) {
    let zone = zone_centree(52, 52, frame.area());
    frame.render_widget(Clear, zone);
    let items: Vec<ListItem> = CHOIX_FORMULAIRES.iter().map(|c| ListItem::new(*c)).collect();
    let mut etat = ListState::default();
    etat.select(Some(choix.min(CHOIX_FORMULAIRES.len() - 1)));
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().title(" Formulaires — Entrée : ouvrir · Échap : fermer ").borders(Borders::ALL).border_style(style_blanc()).title_style(style_titre()))
            .highlight_style(style_selection())
            .highlight_symbol("▸ "),
        zone,
        &mut etat,
    );
}

fn dessiner_formulaire(frame: &mut Frame, form: &Formulaire) {
    let zone = zone_centree(74, 88, frame.area());
    frame.render_widget(Clear, zone);
    frame.render_widget(
        Block::default().title(format!(" {} ", form.titre)).borders(Borders::ALL).border_style(style_titre()).title_style(style_titre()),
        zone,
    );
    let interieur = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            std::iter::repeat_n(Constraint::Length(3), form.champs.len())
                .chain([Constraint::Length(2), Constraint::Length(2)])
                .collect::<Vec<_>>(),
        )
        .split(zone);
    for (i, champ) in form.champs.iter().enumerate() {
        let actif = i == form.actif;
        let valeur = if champ.options.is_empty() {
            if actif { format!("{}▌", champ.valeur) } else { champ.valeur.clone() }
        } else {
            format!("‹ {} ›", champ.valeur)
        };
        frame.render_widget(
            Paragraph::new(valeur)
                .style(if actif { style_selection() } else { style_blanc() })
                .block(
                    Block::default()
                        .title(format!(" {} ", champ.nom))
                        .borders(Borders::ALL)
                        .border_style(if actif { style_selection() } else { style_blanc() }),
                ),
            interieur[i],
        );
    }
    let zone_erreur = interieur[form.champs.len()];
    if let Some(e) = &form.erreur {
        frame.render_widget(Paragraph::new(format!("✗ {e}")).style(style_rgb(theme::ROUGE_RGB).add_modifier(Modifier::BOLD)), zone_erreur);
    }
    frame.render_widget(
        Paragraph::new("Tab/Entrée champ suivant · ←→ choix · Ctrl+S enregistrer · Échap annuler").style(style_violet()),
        interieur[form.champs.len() + 1],
    );
}

// ---------------------------------------------------------------- Ouverture de formulaires

fn ouvrir_formulaire(vue: &mut VueTui, form: Formulaire, module: usize) {
    vue.module = module;
    vue.selection = 0;
    vue.menu_formulaires = false;
    vue.formulaire = Some(form);
}

/// Touche `n` : formulaire de création adapté au module courant.
fn ouvrir_nouveau(store: &mut dyn Store, vue: &mut VueTui) -> Result<(), String> {
    match vue.module {
        0 => {
            let regime = store.societe()?.regime;
            ouvrir_formulaire(vue, Formulaire::societe(regime), 0);
        }
        1 => ouvrir_formulaire(vue, Formulaire::client(), 1),
        2 => ouvrir_formulaire(vue, Formulaire::document(TypeDocument::Facture), 2),
        3 => ouvrir_formulaire(vue, Formulaire::article(), 3),
        _ => ouvrir_formulaire(vue, Formulaire::export_fec(), 4),
    }
    Ok(())
}

/// Entrée du menu `f` (index dans `CHOIX_FORMULAIRES`).
fn ouvrir_choix_menu(store: &mut dyn Store, vue: &mut VueTui, choix: usize) -> Result<(), String> {
    match choix {
        0 => ouvrir_formulaire(vue, Formulaire::client(), 1),
        1 => ouvrir_formulaire(vue, Formulaire::document(TypeDocument::Facture), 2),
        2 => ouvrir_formulaire(vue, Formulaire::document(TypeDocument::Devis), 2),
        3 => ouvrir_formulaire(vue, Formulaire::article(), 3),
        4 => {
            vue.menu_formulaires = false;
            vue.module = 3;
            vue.selection = 0;
            vue.message = "Sélectionnez l'article avec ↑↓ puis Entrée pour le vendre".to_string();
        }
        5 => {
            let regime = store.societe()?.regime;
            ouvrir_formulaire(vue, Formulaire::societe(regime), vue.module);
        }
        _ => ouvrir_formulaire(vue, Formulaire::export_fec(), 4),
    }
    Ok(())
}

fn ouvrir_paiement(store: &mut dyn Store, vue: &VueTui, contenu: &ContenuModule) -> Option<Formulaire> {
    let doc = contenu.factures.get(vue.selection)?;
    let regime = store.societe().ok()?.regime;
    let reste = reste_a_payer(doc.totaux(regime).ttc, doc.paye);
    if reste <= 0 {
        return None;
    }
    Some(Formulaire::paiement(doc, reste))
}

fn ouvrir_vente(vue: &VueTui, contenu: &ContenuModule) -> Option<Formulaire> {
    let article = contenu.articles.get(vue.selection)?;
    if article.statut == "vendu" {
        return None;
    }
    Some(Formulaire::vente(article))
}

// ---------------------------------------------------------------- Boucle Ratatui/Crossterm

/// Boucle principale de la TUI. Retourne une erreur si pas de terminal.
pub fn lancer(store: &mut dyn Store) -> Result<(), String> {
    enable_raw_mode().map_err(|e| format!("terminal en mode raw impossible ({e}) — la TUI requiert un terminal interactif"))?;
    let mut sortie = io::stdout();
    execute!(sortie, EnterAlternateScreen).map_err(|e| e.to_string())?;
    sortie.execute(crossterm::cursor::Hide).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(sortie);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;
    let resultat = boucle(&mut terminal, store);
    // Restauration du terminal quoi qu'il arrive.
    let _ = terminal.backend_mut().execute(crossterm::cursor::Show);
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
    let _ = terminal.show_cursor();
    resultat
}

fn rafraichir(store: &mut dyn Store, vue: &VueTui) -> Result<ContenuModule, String> {
    charger_contenu(store, vue.module)
}

fn boucle(terminal: &mut Terminal<CrosstermBackend<Stdout>>, store: &mut dyn Store) -> Result<(), String> {
    let mut vue = VueTui::default();
    let mut contenu = charger_contenu(store, vue.module)?;
    let nom = store.societe()?.nom;
    loop {
        terminal.draw(|f| dessiner(f, &vue, &contenu, &nom)).map_err(|e| e.to_string())?;
        let Event::Key(touche) = event::read().map_err(|e| e.to_string())? else {
            continue;
        };
        if touche.kind == KeyEventKind::Release {
            continue;
        }

        // 1) Un formulaire ouvert capte toutes les touches.
        if vue.formulaire.is_some() {
            let code = touche.code;
            let ctrl = touche.modifiers.contains(KeyModifiers::CONTROL);
            if code == KeyCode::Esc {
                vue.formulaire = None;
                vue.message = "Formulaire annulé".to_string();
                continue;
            }
            let mut form = vue.formulaire.take().unwrap();
            let enregistrer = (ctrl && code == KeyCode::Char('s')) || (code == KeyCode::Enter && form.est_dernier_champ());
            if enregistrer {
                match soumettre_formulaire(store, &form) {
                    Ok(message) => {
                        vue.message = message;
                        contenu = rafraichir(store, &vue)?;
                    }
                    Err(e) => {
                        form.erreur = Some(e);
                        vue.formulaire = Some(form);
                    }
                }
                continue;
            }
            match code {
                KeyCode::Tab | KeyCode::Down => form.suivant(),
                KeyCode::BackTab | KeyCode::Up => form.precedent(),
                KeyCode::Enter => form.suivant(),
                KeyCode::Left => form.defiler_choix(-1),
                KeyCode::Right => form.defiler_choix(1),
                KeyCode::Backspace => form.effacer(),
                KeyCode::Char(c) if !ctrl => form.saisir(c),
                _ => {}
            }
            vue.formulaire = Some(form);
            continue;
        }

        // 2) Le menu des formulaires (touche f), s'il est ouvert.
        if vue.menu_formulaires {
            match touche.code {
                KeyCode::Esc => vue.menu_formulaires = false,
                KeyCode::Up | KeyCode::Char('k') => {
                    vue.choix_menu = (vue.choix_menu + CHOIX_FORMULAIRES.len() - 1) % CHOIX_FORMULAIRES.len()
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    vue.choix_menu = (vue.choix_menu + 1) % CHOIX_FORMULAIRES.len()
                }
                KeyCode::Enter => {
                    let choix = vue.choix_menu;
                    ouvrir_choix_menu(store, &mut vue, choix)?;
                    if vue.formulaire.is_some() {
                        contenu = rafraichir(store, &vue)?;
                    } else if vue.module == 3 {
                        contenu = rafraichir(store, &vue)?;
                    }
                }
                _ => {}
            }
            continue;
        }

        // 3) L'aide, si elle est affichée.
        if vue.aide {
            if matches!(touche.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
                vue.aide = false;
            }
            continue;
        }

        // 4) Navigation et actions générales.
        match touche.code {
            KeyCode::Char('q') => break,
            KeyCode::Char('c') if touche.modifiers.contains(KeyModifiers::CONTROL) => break,
            KeyCode::Char('?') => vue.aide = true,
            KeyCode::Char('f') => {
                vue.menu_formulaires = true;
                vue.choix_menu = 0;
            }
            KeyCode::Tab => {
                vue.module = (vue.module + 1) % MODULES.len();
                vue.selection = 0;
                contenu = rafraichir(store, &vue)?;
            }
            KeyCode::BackTab => {
                vue.module = (vue.module + MODULES.len() - 1) % MODULES.len();
                vue.selection = 0;
                contenu = rafraichir(store, &vue)?;
            }
            KeyCode::Up | KeyCode::Char('k') => vue.selection = vue.selection.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if vue.selection + 1 < contenu.lignes.len() {
                    vue.selection += 1;
                }
            }
            KeyCode::Char('r') => {
                contenu = rafraichir(store, &vue)?;
                vue.message = "Données rafraîchies".to_string();
            }
            KeyCode::Char('n') => ouvrir_nouveau(store, &mut vue)?,
            KeyCode::Char('d') if vue.module == 2 => {
                ouvrir_formulaire(&mut vue, Formulaire::document(TypeDocument::Devis), 2);
            }
            KeyCode::Char('s') if vue.module == 4 => {
                let regime = store.societe()?.regime;
                ouvrir_formulaire(&mut vue, Formulaire::societe(regime), 4);
            }
            KeyCode::Char('x') if vue.module == 4 => ouvrir_formulaire(&mut vue, Formulaire::export_fec(), 4),
            KeyCode::Char('v') if vue.module == 2 => {
                if let Some(d) = contenu.factures.get(vue.selection).cloned() {
                    if !peut_modifier(d.statut) {
                        vue.message = format!("✗ {} est déjà {} : un document validé ne se modifie plus", d.numero, d.statut.as_str());
                    } else {
                        match store.changer_statut(&d.numero, StatutDocument::Valide) {
                            Ok(_) => {
                                let _ = store.journaliser("tui.validation", &d.numero);
                                vue.message = format!("✓ {} validée", d.numero);
                            }
                            Err(e) => vue.message = format!("✗ {e}"),
                        }
                        contenu = rafraichir(store, &vue)?;
                    }
                }
            }
            KeyCode::Char('p') if vue.module == 2 => {
                if contenu.factures.get(vue.selection).is_none() {
                    vue.message = "Aucune facture sélectionnée".to_string();
                } else if let Some(form) = ouvrir_paiement(store, &vue, &contenu) {
                    vue.formulaire = Some(form);
                } else {
                    let d = &contenu.factures[vue.selection];
                    vue.message = format!("{} est déjà soldée", d.numero);
                }
            }
            KeyCode::Enter if vue.module == 3 => {
                if contenu.articles.get(vue.selection).is_none() {
                    vue.message = "Aucun article sélectionné".to_string();
                } else if let Some(form) = ouvrir_vente(&vue, &contenu) {
                    vue.formulaire = Some(form);
                } else {
                    vue.message = "Cet article est déjà vendu".to_string();
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn contenu_ecran(vue: &VueTui, contenu: &ContenuModule) -> String {
        let backend = TestBackend::new(110, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| dessiner(f, vue, contenu, "Ma micro-entreprise")).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn rendu_ratatui_modules_et_lignes() {
        let vue = VueTui { module: 2, selection: 0, ..VueTui::default() };
        let contenu = ContenuModule {
            lignes: vec!["✓ FA-2026-0001  TTC 1 000,00 €".into()],
            factures: Vec::new(),
            clients: Vec::new(),
            articles: Vec::new(),
        };
        let ecran = contenu_ecran(&vue, &contenu);
        assert!(ecran.contains("Factures"));
        assert!(ecran.contains("FA-2026-0001"));
        assert!(ecran.contains("Ma micro-entreprise"));
        assert!(ecran.contains("Tableau de bord"));
        // Aucun style Ratatui de la TUI ne doit définir de fond coloré.
        let backend = TestBackend::new(110, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| dessiner(f, &vue, &contenu, "Ma micro-entreprise")).unwrap();
        assert!(terminal.backend().buffer().content().iter().all(|c| c.bg == Color::Reset));
    }

    #[test]
    fn rendu_formulaire_client() {
        let vue = VueTui { module: 1, formulaire: Some(Formulaire::client()), ..VueTui::default() };
        let contenu = ContenuModule::vide(Vec::new());
        let ecran = contenu_ecran(&vue, &contenu);
        assert!(ecran.contains("Nouveau client"));
        assert!(ecran.contains("Nom"));
        assert!(ecran.contains("Ctrl+S"));
    }

    #[test]
    fn navigation_formulaire_et_choix() {
        let mut form = Formulaire::client();
        form.suivant();
        assert_eq!(form.actif, 1); // champ Type (à choix)
        form.defiler_choix(1);
        assert_eq!(form.champs[1].valeur, "particulier");
        form.defiler_choix(1);
        assert_eq!(form.champs[1].valeur, "pro");
        form.precedent();
        form.saisir('Z');
        assert_eq!(form.champs[0].valeur, "Z");
        form.effacer();
        assert_eq!(form.champs[0].valeur, "");
    }

    #[test]
    fn formulaire_client_cree_un_client() {
        let mut s = Memoire::new();
        let mut form = Formulaire::client();
        form.champs[0].valeur = "Atelier Dupont".into();
        form.champs[2].valeur = "contact@example.fr".into();
        let msg = soumettre_formulaire(&mut s, &form).unwrap();
        assert!(msg.contains("Atelier Dupont"));
        assert_eq!(s.clients().unwrap().len(), 1);
    }

    #[test]
    fn formulaire_client_sans_nom_refuse() {
        let mut s = Memoire::new();
        let form = Formulaire::client();
        assert!(soumettre_formulaire(&mut s, &form).is_err());
        assert!(s.clients().unwrap().is_empty());
    }

    #[test]
    fn formulaire_facture_puis_paiement() {
        let mut s = Memoire::new();
        s.creer_client(Client {
            id: 0,
            nom: "Atelier Dupont".into(),
            type_client: TypeClient::Professionnel,
            email: String::new(),
            telephone: String::new(),
            adresse: String::new(),
            siren: String::new(),
            tva_intracom: String::new(),
            notes: String::new(),
        })
        .unwrap();
        let mut form = Formulaire::document(TypeDocument::Facture);
        form.champs[0].valeur = "dupont".into();
        form.champs[4].valeur = "Maintenance".into();
        form.champs[5].valeur = "2".into();
        form.champs[6].valeur = "500,00".into();
        let msg = soumettre_formulaire(&mut s, &form).unwrap();
        assert!(msg.contains("FA-"));
        let doc = s.documents(Some(TypeDocument::Facture)).unwrap().remove(0);
        s.changer_statut(&doc.numero, StatutDocument::Valide).unwrap();
        let pay = Formulaire::paiement(&doc, 100_000);
        let msg = soumettre_formulaire(&mut s, &pay).unwrap();
        assert!(msg.contains("Paiement enregistré"));
        let doc = s.document_par_numero(&doc.numero).unwrap();
        assert_eq!(doc.statut, StatutDocument::Paye);
    }

    #[test]
    fn formulaire_article_puis_vente() {
        let mut s = Memoire::new();
        let mut form = Formulaire::article();
        form.champs[0].valeur = "Zara".into();
        form.champs[1].valeur = "Veste".into();
        form.champs[3].valeur = "100 % coton".into();
        form.champs[5].valeur = "8,00".into();
        soumettre_formulaire(&mut s, &form).unwrap();
        let article = s.articles().unwrap().remove(0);
        let mut vente = Formulaire::vente(&article);
        vente.champs[0].valeur = "25,00".into();
        vente.champs[1].valeur = "2,00".into();
        let msg = soumettre_formulaire(&mut s, &vente).unwrap();
        assert!(msg.contains("marge nette 15,00 €"));
        assert_eq!(s.article_par_id(article.id).unwrap().statut, "vendu");
    }

    #[test]
    fn formulaire_societe_change_le_regime() {
        let mut s = Memoire::new();
        let mut form = Formulaire::societe(RegimeTva::Franchise);
        form.defiler_choix(1);
        soumettre_formulaire(&mut s, &form).unwrap();
        assert_eq!(s.societe().unwrap().regime, RegimeTva::Reel);
    }

    #[test]
    fn contenu_tableau_de_bord_en_memoire() {
        let mut s = Memoire::new();
        let c = charger_contenu(&mut s, 0).unwrap();
        assert!(c.lignes.iter().any(|l| l.contains("CA global")));
    }
}
