//! Interface plein écran (TUI) — Ratatui + Crossterm (ADR-005).
//!
//! La TUI n'a **aucune règle métier propre** : chaque écran est une commande
//! du CLI rendue dans un cadre. Elle appelle [`App::apercu`] en lecture seule
//! pour afficher (aucune écriture, aucun journal d'audit pollué par la
//! navigation) et [`App::executer`] pour les actions — ce que fait un
//! formulaire n'est rien d'autre que composer une ligne de commande, ce qui
//! garantit que la TUI et le CLI ne peuvent pas diverger (CDC §6, ENF-ERG-02).
//!
//! Raccourcis affichés en permanence en pied d'écran (CDC §6.2) :
//! `←→` onglets · `↑↓` défilement · `n` nouveau · `p` paiement · `s` recherche
//! · `r` rafraîchir · `?` aide · `q` quitter. Aucune action destructive n'est
//! accessible sans passer par la commande explicite (le CLI demande la
//! référence), donc aucune ne peut être déclenchée par une touche seule.

use crate::cli::App;
use crate::theme;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TxtLine, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io::Stdout;
use std::time::Duration;

/// Un onglet = un chemin du CLI. L'ordre suit le parcours quotidien.
pub const ONGLETS: [(&str, &str); 8] = [
    ("Tableau de bord", "/"),
    ("Clients", "/clients"),
    ("Devis", "/devis"),
    ("Factures", "/factures"),
    ("Avoirs", "/avoirs"),
    ("Vinted", "/vinted"),
    // `/compta` seul demande un sous-chemin : l'onglet ouvre le tableau de bord
    // financier, qui est l'écran utile au quotidien (EF-CPT-05).
    ("Compta", "/compta tableau"),
    ("Société", "/societes"),
];

/// Ligne de raccourcis affichée en permanence (CDC §6.2).
pub const RACCOURCIS: &str =
    "←→ onglets · ↑↓ défiler · n nouveau · p paiement · s recherche · r rafraîchir · ? aide · q quitter";

// ------------------------------------------------------------------ Formulaire

/// Un champ de saisie. `cle` sert d'identifiant dans le modèle de commande.
#[derive(Debug, Clone)]
pub struct Champ {
    pub cle: &'static str,
    pub libelle: &'static str,
    pub valeur: String,
    pub obligatoire: bool,
    pub indice: &'static str,
}

impl Champ {
    pub fn new(
        cle: &'static str,
        libelle: &'static str,
        obligatoire: bool,
        indice: &'static str,
    ) -> Self {
        Self {
            cle,
            libelle,
            valeur: String::new(),
            obligatoire,
            indice,
        }
    }

    /// Valeur nettoyée : les guillemets sont retirés (ils casseraient le
    /// découpage de la ligne de commande), les espaces de bord aussi.
    pub fn propre(&self) -> String {
        self.valeur
            .replace('"', "")
            .trim()
            .to_string()
    }
}

/// Formulaire = modèle de ligne de commande + champs.
///
/// `modele` contient des jetons `{cle}` remplacés par la saisie. Après
/// remplacement, un `{…}` restant signale un champ obligatoire vide : le
/// message d'erreur nomme alors le champ, en français.
#[derive(Debug, Clone)]
pub struct Formulaire {
    pub titre: &'static str,
    pub modele: &'static str,
    pub champs: Vec<Champ>,
    pub focus: usize,
    pub erreur: Option<String>,
}

impl Formulaire {
    pub fn nouveau(titre: &'static str, modele: &'static str, champs: Vec<Champ>) -> Self {
        Self {
            titre,
            modele,
            champs,
            focus: 0,
            erreur: None,
        }
    }

    /// Formulaire de création de client **ou de prospect** (EF-CRM-01).
    /// Le statut est pré-rempli à « client » : appuyer sur Entrée crée un
    /// client, taper « prospect » crée une piste commerciale.
    pub fn client() -> Self {
        let mut form = Self::nouveau(
            "Nouveau client / prospect",
            "/clients nouveau --nom \"{nom}\" --type {type} --statut {statut} \
             --email \"{email}\" --ville \"{ville}\"",
            vec![
                Champ::new("nom", "Nom", true, "raison sociale ou nom+prénom"),
                Champ::new("type", "Type", true, "particulier | professionnel | association"),
                Champ::new("statut", "Statut", false, "client (défaut) ou prospect"),
                Champ::new("email", "E-mail", false, "facultatif, sert à l'envoi"),
                Champ::new("ville", "Ville", false, "facultatif"),
            ],
        );
        form.champs[2].valeur = "client".to_string();
        form
    }

    /// Formulaire de création de document (devis ou facture).
    pub fn document(type_doc: &str) -> Self {
        let titre = if type_doc == "devis" {
            "Nouveau devis"
        } else {
            "Nouvelle facture"
        };
        Self::nouveau(
            titre,
            if type_doc == "devis" {
                "/devis nouveau --client \"{client}\" --activite {activite} --ligne \"{ligne}\""
            } else {
                "/factures nouveau --client \"{client}\" --activite {activite} --ligne \"{ligne}\""
            },
            vec![
                Champ::new("client", "Client", true, "nom, e-mail ou n°"),
                Champ::new(
                    "activite",
                    "Activité",
                    true,
                    "services (BNC) | ventes (BIC)",
                ),
                Champ::new(
                    "ligne",
                    "Ligne",
                    true,
                    "désignation;quantité;prix HT (ex. Prestation;1;750,00)",
                ),
            ],
        )
    }

    /// Formulaire d'enregistrement de paiement (EF-FAC-06).
    pub fn paiement() -> Self {
        Self::nouveau(
            "Enregistrer un paiement",
            "/factures {numero} payer {montant} {date} {moyen}",
            vec![
                Champ::new("numero", "Facture", true, "FA-2026-0001 ou 2026/0001"),
                Champ::new("montant", "Montant", true, "vide = solde restant"),
                Champ::new("date", "Date", false, "vide = aujourd'hui"),
                Champ::new("moyen", "Moyen", false, "virement | cb | espèces | chèque"),
            ],
        )
    }

    /// Formulaire d'ajout d'article Vinted (EF-VIN-01).
    pub fn article() -> Self {
        Self::nouveau(
            "Nouvel article Vinted",
            "/vinted nouveau --marque \"{marque}\" --description \"{description}\" \
             --taille {taille} --composition \"{composition}\" --prix-achat {prix_achat}",
            vec![
                Champ::new("marque", "Marque", true, "ex. Patagonia"),
                Champ::new("description", "Description", true, "ex. Veste polaire"),
                Champ::new("taille", "Taille", false, "ex. M"),
                Champ::new(
                    "composition",
                    "Composition",
                    true,
                    "obligatoire (UE 1007/2011), ex. 100 % polyester",
                ),
                Champ::new("prix_achat", "Prix d'achat", true, "ex. 12,00"),
            ],
        )
    }

    /// Formulaire de vente d'un article (EF-VIN-02).
    pub fn vente() -> Self {
        Self::nouveau(
            "Vendre un article",
            "/vinted vendre {article} --prix {prix} --frais {frais}",
            vec![
                Champ::new("article", "N° article", true, "visible dans /vinted"),
                Champ::new("prix", "Prix de vente", true, "ex. 45,00"),
                Champ::new("frais", "Frais Vinted", false, "vide = 0,00"),
            ],
        )
    }

    /// Formulaire de recherche globale (EF-SOC-03).
    pub fn recherche() -> Self {
        Self::nouveau(
            "Recherche globale",
            "/recherche {texte}",
            vec![Champ::new("texte", "Texte", true, "client, facture, article, vente")],
        )
    }

    /// Ligne de commande équivalente à la saisie (vide si un champ
    /// obligatoire manque — le message nomme le libellé, pas la clé).
    pub fn commande(&self) -> Result<String, String> {
        let mut ligne = self.modele.to_string();
        for champ in &self.champs {
            let jeton = format!("{{{}}}", champ.cle);
            let valeur = champ.propre();
            if valeur.is_empty() {
                if champ.obligatoire {
                    return Err(format!("« {} » est obligatoire — {}", champ.libelle, champ.indice));
                }
                // Champ facultatif vide : on retire son option et sa valeur,
                // avec ou sans guillemets dans le modèle.
                ligne = ligne.replace(&format!(" --{} \"{jeton}\"", champ.cle), "");
                ligne = ligne.replace(&format!(" --{} {jeton}", champ.cle), "");
                ligne = ligne.replace(&format!(" {jeton}"), "");
                continue;
            }
            ligne = ligne.replace(&jeton, &valeur);
        }
        let restant = ligne
            .find('{')
            .and_then(|debut| ligne[debut..].find('}').map(|fin| &ligne[debut..debut + fin + 1]));
        if let Some(jeton) = restant {
            let cle = jeton.trim_matches(|c| c == '{' || c == '}');
            let libelle = self
                .champs
                .iter()
                .find(|c| c.cle == cle)
                .map(|c| c.libelle)
                .unwrap_or(cle);
            return Err(format!("« {libelle} » est obligatoire"));
        }
        Ok(ligne.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    pub fn avancer(&mut self) {
        if !self.champs.is_empty() {
            // `focus` sert aussi de « champ actif » pour la saisie clavier.
            self.focus = (self.focus + 1) % self.champs.len();
        }
    }

    pub fn reculer(&mut self) {
        if !self.champs.is_empty() {
            self.focus = (self.focus + self.champs.len() - 1) % self.champs.len();
        }
    }

    pub fn saisir(&mut self, c: char) {
        if let Some(champ) = self.champs.get_mut(self.focus) {
            champ.valeur.push(c);
        }
    }

    pub fn effacer(&mut self) {
        if let Some(champ) = self.champs.get_mut(self.focus) {
            champ.valeur.pop();
        }
    }
}

// ---------------------------------------------------------------------- Vue

/// État de l'écran : onglet, défilement, contenu déjà chargé.
#[derive(Debug, Clone)]
pub struct Vue {
    pub onglet: usize,
    pub titre: String,
    pub lignes: Vec<String>,
    pub defilement: u16,
    pub message: String,
    pub succes: bool,
    pub formulaire: Option<Formulaire>,
    pub aide: bool,
    pub recherhe: Option<String>,
    pub quitter: bool,
}

impl Default for Vue {
    fn default() -> Self {
        Self {
            onglet: 0,
            titre: String::new(),
            lignes: Vec::new(),
            defilement: 0,
            message: "Prêt · toutes les données restent locales".to_string(),
            succes: false,
            formulaire: None,
            aide: false,
            recherhe: None,
            quitter: false,
        }
    }
}

impl Vue {
    pub fn nouvelle() -> Self {
        Self::default()
    }

    /// Commande correspondant à l'écran courant.
    pub fn chemin(&self) -> String {
        match &self.recherhe {
            Some(texte) => format!("/recherche {texte}"),
            None => ONGLETS[self.onglet.min(ONGLETS.len() - 1)].1.to_string(),
        }
    }

    /// Recharge le contenu depuis l'application (lecture seule).
    pub fn rafraichir(&mut self, app: &mut App) {
        let commande = self.chemin();
        let mut commande = if commande.is_empty() {
            "/".to_string()
        } else {
            commande
        };
        if commande.starts_with("/clients")
            || commande.starts_with("/devis")
            || commande.starts_with("/factures")
            || commande.starts_with("/avoirs")
            || commande.starts_with("/vinted")
            || commande.starts_with("/compta")
        {
            commande.push_str(" --lignes 500");
        }
        match app.apercu(&commande) {
            Ok(texte) => {
                self.lignes = sans_ansi(&texte)
                    .lines()
                    .map(|l| l.trim_end().to_string())
                    .collect();
                if self.defilement as usize >= self.lignes.len() {
                    self.defilement = self.lignes.len().saturating_sub(1) as u16;
                }
                self.titre = ONGLETS[self.onglet.min(ONGLETS.len() - 1)].0.to_string();
            }
            Err(e) => {
                self.lignes = vec![format!("✗ {e}")];
                self.message = e;
                self.succes = false;
            }
        }
    }

    /// Ouvre le formulaire attendu pour l'onglet courant.
    pub fn formulaire_onglet(&self) -> Formulaire {
        match ONGLETS[self.onglet.min(ONGLETS.len() - 1)].1 {
            "/clients" => Formulaire::client(),
            "/devis" => Formulaire::document("devis"),
            "/factures" | "/avoirs" => Formulaire::document("facture"),
            "/vinted" => Formulaire::article(),
            _ => Formulaire::recherche(),
        }
    }

    /// Applique une touche. Renvoie `true` si un rafraîchissement est utile.
    pub fn touche(&mut self, app: &mut App, touche: KeyEvent) -> bool {
        if touche.kind != KeyEventKind::Press {
            return false;
        }
        // 1. Saisie dans un formulaire.
        if let Some(mut form) = self.formulaire.take() {
            match touche.code {
                KeyCode::Esc => {
                    self.message = "Saisie abandonnée".to_string();
                    self.succes = false;
                }
                KeyCode::Tab | KeyCode::Down => {
                    form.avancer();
                    self.formulaire = Some(form);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    form.reculer();
                    self.formulaire = Some(form);
                }
                KeyCode::Backspace => {
                    form.effacer();
                    self.formulaire = Some(form);
                }
                KeyCode::Enter => match form.commande() {
                    Err(e) => {
                        form.erreur = Some(e);
                        self.formulaire = Some(form);
                    }
                    Ok(commande) => {
                        let mut tampon: Vec<u8> = Vec::new();
                        match app.executer(&commande, &mut tampon) {
                            Ok(_) => {
                                self.message = sans_ansi(&String::from_utf8_lossy(&tampon))
                                    .lines()
                                    .next()
                                    .unwrap_or("fait")
                                    .to_string();
                                self.succes = true;
                                self.recherhe = None;
                                return true;
                            }
                            Err(e) => {
                                self.message = e.clone();
                                form.erreur = Some(e);
                                self.formulaire = Some(form);
                                self.succes = false;
                            }
                        }
                    }
                },
                KeyCode::Char(c) => {
                    form.saisir(c);
                    self.formulaire = Some(form);
                }
                _ => self.formulaire = Some(form),
            }
            return false;
        }
        // 2. Aide affichée : n'importe quelle touche la referme.
        if self.aide {
            self.aide = false;
            return false;
        }
        // 3. Navigation.
        match touche.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quitter = true,
            KeyCode::Char('c') if touche.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quitter = true
            }
            KeyCode::Right | KeyCode::Tab => {
                self.onglet = (self.onglet + 1) % ONGLETS.len();
                self.defilement = 0;
                self.recherhe = None;
                return true;
            }
            KeyCode::Left | KeyCode::BackTab => {
                self.onglet = (self.onglet + ONGLETS.len() - 1) % ONGLETS.len();
                self.defilement = 0;
                self.recherhe = None;
                return true;
            }
            KeyCode::Down => {
                if (self.defilement as usize) + 1 < self.lignes.len().max(1) {
                    self.defilement += 1;
                }
            }
            KeyCode::Up => self.defilement = self.defilement.saturating_sub(1),
            KeyCode::PageDown => {
                self.defilement = (self.defilement + 10).min(self.lignes.len() as u16);
            }
            KeyCode::PageUp => self.defilement = self.defilement.saturating_sub(10),
            KeyCode::Home => self.defilement = 0,
            KeyCode::End => {
                self.defilement = self.lignes.len().saturating_sub(1) as u16;
            }
            KeyCode::Char('r') | KeyCode::F(5) => return true,
            KeyCode::Char('n') => {
                self.formulaire = Some(self.formulaire_onglet());
            }
            KeyCode::Char('p') => {
                self.formulaire = Some(Formulaire::paiement());
            }
            KeyCode::Char('v') => {
                self.formulaire = Some(Formulaire::vente());
            }
            KeyCode::Char('s') | KeyCode::Char('/') => {
                self.formulaire = Some(Formulaire::recherche());
            }
            KeyCode::Char('?') => self.aide = true,
            _ => {}
        }
        false
    }
}

/// Retire les séquences ANSI (le CLI colore, Ratatui ne les interprète pas).
pub fn sans_ansi(texte: &str) -> String {
    let mut sortie = String::with_capacity(texte.len());
    let mut iter = texte.chars().peekable();
    while let Some(c) = iter.next() {
        if c == '\u{1b}' {
            if iter.peek() == Some(&'[') {
                iter.next();
                // Jusqu'à la lettre finale (m pour une couleur, K pour un effacement).
                for suivant in iter.by_ref() {
                    if suivant.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        sortie.push(c);
    }
    sortie
}

/// Style d'une ligne de contenu, d'après son marqueur sémantique.
fn style_ligne(ligne: &str) -> Style {
    let base = Style::default().fg(rgb(theme::BLANC_RGB));
    if ligne.starts_with('✓') {
        base.fg(rgb(theme::VERT_RGB))
    } else if ligne.starts_with('⚠') {
        base.fg(rgb(theme::ORANGE_RGB))
    } else if ligne.starts_with('✗') {
        base.fg(rgb(theme::ROUGE_RGB))
    } else if ligne.starts_with("────") || ligne.starts_with("─") {
        base.fg(rgb(theme::VIOLET_INACTIF_RGB))
    } else if ligne.starts_with("page ") || ligne.starts_with("(") {
        base.fg(rgb(theme::BLEU_RGB))
    } else {
        base
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

fn style_titre() -> Style {
    Style::default()
        .fg(rgb(theme::BLANC_RGB))
        .add_modifier(Modifier::BOLD)
}

fn style_violet() -> Style {
    Style::default().fg(rgb(theme::VIOLET_RGB))
}

fn style_onglet_actif() -> Style {
    Style::default()
        .fg(rgb(theme::BLANC_RGB))
        .bg(rgb(theme::SURLIGNAGE_BLANC_20_RGB))
        .add_modifier(Modifier::BOLD)
}

fn style_onglet_inactif() -> Style {
    Style::default().fg(rgb(theme::VIOLET_INACTIF_RGB))
}

fn style_message(vue: &Vue) -> Style {
    if vue.succes {
        Style::default().fg(rgb(theme::VERT_RGB))
    } else {
        Style::default().fg(rgb(theme::ORANGE_RGB))
    }
}

// ------------------------------------------------------------------ Rendu

fn dessiner(frame: &mut Frame, vue: &Vue, societe: &str, regime: &str, devise: &str) {
    let zones = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    // En-tête : société, régime, devise, chemin courant.
    let entete = TxtLine::from(vec![
        Span::styled(" ERP ", style_onglet_actif()),
        Span::styled(format!(" {societe} "), style_titre()),
        Span::styled(
            format!("· {regime} · {devise} · {} ", vue.chemin().trim()),
            style_violet(),
        ),
    ]);
    frame.render_widget(Paragraph::new(entete), zones[0]);

    // Onglets.
    let mut onglets: Vec<Span> = Vec::new();
    for (index, (nom, _)) in ONGLETS.iter().enumerate() {
        let style = if index == vue.onglet {
            style_onglet_actif()
        } else {
            style_onglet_inactif()
        };
        onglets.push(Span::styled(format!(" {nom} "), style));
        onglets.push(Span::styled("│", style_onglet_inactif()));
    }
    frame.render_widget(Paragraph::new(TxtLine::from(onglets)), zones[1]);

    // Contenu.
    let lignes: Vec<TxtLine> = vue
        .lignes
        .iter()
        .map(|l| TxtLine::from(Span::styled(l.clone(), style_ligne(l))))
        .collect();
    let bloc = Block::default()
        .borders(Borders::ALL)
        .border_style(style_violet())
        .title(Span::styled(
            format!(" {} ", vue.titre),
            style_titre(),
        ));
    let contenu = Paragraph::new(lignes)
        .block(bloc)
        .wrap(Wrap { trim: false })
        .scroll((vue.defilement, 0));
    frame.render_widget(contenu, zones[2]);

    // Pied : message + raccourcis (toujours visibles).
    let pied = TxtLine::from(vec![
        Span::styled(format!(" {} ", vue.message), style_message(vue)),
        Span::styled(RACCOURCIS, style_onglet_inactif()),
    ]);
    frame.render_widget(Paragraph::new(pied), zones[3]);

    if vue.aide {
        dessiner_aide(frame, vue);
    }
    if let Some(form) = &vue.formulaire {
        dessiner_formulaire(frame, form);
    }
}

fn dessiner_aide(frame: &mut Frame, vue: &Vue) {
    let zone = centre(frame.area(), 80, 70);
    frame.render_widget(Clear, zone);
    let mut lignes = vec![
        TxtLine::from(Span::styled("Aide contextuelle", style_titre())),
        TxtLine::from(""),
        TxtLine::from(format!("Écran : {}", vue.chemin())),
        TxtLine::from(""),
    ];
    for (nom, chemin) in ONGLETS {
        lignes.push(TxtLine::from(format!("  {nom:<18} {chemin}")));
    }
    lignes.push(TxtLine::from(""));
    lignes.push(TxtLine::from(RACCOURCIS));
    lignes.push(TxtLine::from(""));
    lignes.push(TxtLine::from(
        "Toute commande du CLI s'exécute aussi ici : tapez-la dans un terminal.",
    ));
    let bloc = Block::default()
        .borders(Borders::ALL)
        .border_style(style_violet())
        .title(Span::styled(" ? ", style_titre()));
    frame.render_widget(
        Paragraph::new(lignes)
            .block(bloc)
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Left),
        zone,
    );
}

fn dessiner_formulaire(frame: &mut Frame, form: &Formulaire) {
    let hauteur = (form.champs.len() as u16 * 3 + 6).min(frame.area().height);
    let zone = centre_hauteur(frame.area(), 80, hauteur);
    frame.render_widget(Clear, zone);

    let mut lignes: Vec<TxtLine> = Vec::new();
    for (index, champ) in form.champs.iter().enumerate() {
        let actif = index == form.focus;
        let style = if actif {
            style_onglet_actif()
        } else {
            Style::default().fg(rgb(theme::BLANC_RGB))
        };
        let marque = if champ.obligatoire { "*" } else { " " };
        lignes.push(TxtLine::from(Span::styled(
            format!("{marque} {}", champ.libelle),
            style_titre(),
        )));
        lignes.push(TxtLine::from(Span::styled(
            format!("  {}▏", champ.valeur),
            style,
        )));
        lignes.push(TxtLine::from(Span::styled(
            format!("  {}", champ.indice),
            Style::default().fg(rgb(theme::VIOLET_INACTIF_RGB)),
        )));
    }
    if let Some(erreur) = &form.erreur {
        lignes.push(TxtLine::from(Span::styled(
            format!("✗ {erreur}"),
            Style::default().fg(rgb(theme::ROUGE_RGB)),
        )));
    }
    lignes.push(TxtLine::from(Span::styled(
        "Entrée valider · Tab champ suivant · Échap annuler",
        style_onglet_inactif(),
    )));

    let bloc = Block::default()
        .borders(Borders::ALL)
        .border_style(style_violet())
        .title(Span::styled(format!(" {} ", form.titre), style_titre()));
    frame.render_widget(
        Paragraph::new(lignes)
            .block(bloc)
            .wrap(Wrap { trim: false }),
        zone,
    );
}

fn centre(zone: Rect, pourcent_largeur: u16, pourcent_hauteur: u16) -> Rect {
    let hauteur = zone.height * pourcent_hauteur / 100;
    centre_hauteur(zone, pourcent_largeur, hauteur)
}

fn centre_hauteur(zone: Rect, pourcent_largeur: u16, hauteur: u16) -> Rect {
    let largeur = zone.width * pourcent_largeur / 100;
    let largeur = largeur.min(zone.width);
    let hauteur = hauteur.min(zone.height);
    Rect {
        x: zone.x + (zone.width.saturating_sub(largeur)) / 2,
        y: zone.y + (zone.height.saturating_sub(hauteur)) / 2,
        width: largeur,
        height: hauteur,
    }
}

// ------------------------------------------------------------------ Boucle

/// Lance la TUI sur le magasin déjà ouvert par `main`.
///
/// Le terminal est toujours restauré, même si une touche provoque une erreur :
/// une session interrompue ne doit pas laisser le terminal en mode brut.
pub fn lancer(app: &mut App) -> Result<(), String> {
    let mut sortie = std::io::stdout();
    enable_raw_mode().map_err(|e| format!("passage du terminal en mode brut : {e}"))?;
    if let Err(e) = execute!(sortie, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(format!("écran alternatif : {e}"));
    }
    let backend = CrosstermBackend::new(sortie);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            return Err(format!("initialisation du terminal : {e}"));
        }
    };
    let resultat = boucle(&mut terminal, app);
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    resultat
}

fn boucle(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<(), String> {
    let mut vue = Vue::nouvelle();
    vue.rafraichir(app);
    loop {
        let mut nom = "Société".to_string();
        let mut regime = String::new();
        let mut devise = String::new();
        if let Ok(societe) = app.store.societe() {
            nom = societe.nom.clone();
            regime = societe.regime.libelle().to_string();
            devise = societe.devise.clone();
        }
        terminal
            .draw(|frame| dessiner(frame, &vue, &nom, &regime, &devise))
            .map_err(|e| format!("affichage : {e}"))?;
        if vue.quitter {
            return Ok(());
        }
        if !event::poll(Duration::from_millis(250)).map_err(|e| format!("terminal : {e}"))? {
            continue;
        }
        if let Event::Key(touche) = event::read().map_err(|e| format!("lecture clavier : {e}"))? {
            if vue.touche(app, touche) {
                vue.rafraichir(app);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lancer_cli(app: &mut App, ligne: &str) -> String {
        let mut tampon: Vec<u8> = Vec::new();
        app.executer(ligne, &mut tampon)
            .unwrap_or_else(|e| panic!("{ligne} : {e}"));
        sans_ansi(&String::from_utf8_lossy(&tampon).to_string())
    }

    #[test]
    fn les_sequences_ansi_sont_retirees() {
        let texte = "\u{1b}[38;2;74;222;128m✓ créé\u{1b}[0m\nligne 2";
        assert_eq!(sans_ansi(texte), "✓ créé\nligne 2");
        assert_eq!(sans_ansi("sans couleur"), "sans couleur");
    }

    #[test]
    fn chaque_onglet_a_un_chemin_du_cli() {
        for (nom, chemin) in ONGLETS {
            assert!(!nom.is_empty());
            assert!(chemin.starts_with('/'), "{chemin}");
        }
        let mut vue = Vue::nouvelle();
        vue.onglet = 2;
        assert_eq!(vue.chemin(), "/devis");
        vue.onglet = 5;
        assert_eq!(vue.chemin(), "/vinted");
    }

    #[test]
    fn un_formulaire_compose_une_commande_du_cli() {
        let mut form = Formulaire::client();
        form.champs[0].valeur = "Dupont SARL".to_string();
        form.champs[1].valeur = "professionnel".to_string();
        // « Dupont SARL » contient une espace : elle doit être préservée
        // (le CLI découpe les guillemets lui-même). Le statut est pré-rempli.
        let commande = form.commande().unwrap();
        assert_eq!(
            commande,
            "/clients nouveau --nom \"Dupont SARL\" --type professionnel --statut client"
        );
        // Le même formulaire crée un prospect.
        form.champs[2].valeur = "prospect".to_string();
        let commande = form.commande().unwrap();
        assert!(commande.ends_with("--statut prospect"), "{commande}");
    }

    #[test]
    fn un_champ_obligatoire_vide_est_signale_en_francais() {
        let form = Formulaire::client();
        let erreur = form.commande().unwrap_err();
        assert!(erreur.contains("Nom"), "{erreur}");
        assert!(erreur.contains("obligatoire"), "{erreur}");
    }

    #[test]
    fn un_formulaire_valide_cree_vraiment_un_client() {
        let mut app = App::memoire();
        let mut form = Formulaire::client();
        form.champs[0].valeur = "Atelier Dupont".to_string();
        form.champs[1].valeur = "professionnel".to_string();
        form.champs[4].valeur = "Nice".to_string();
        let commande = form.commande().unwrap();
        let mut tampon: Vec<u8> = Vec::new();
        app.executer(&commande, &mut tampon).unwrap();
        let liste = lancer_cli(&mut app, "/clients");
        assert!(liste.contains("Atelier Dupont"), "{liste}");
    }

    #[test]
    fn la_touche_n_ouvre_le_formulaire_de_l_onglet() {
        let mut app = App::memoire();
        let mut vue = Vue::nouvelle();
        vue.onglet = 1; // clients
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(
            vue.formulaire.as_ref().map(|f| f.modele),
            Some(Formulaire::client().modele)
        );
        vue.formulaire = None;
        vue.onglet = 3; // factures
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('n')));
        assert!(vue
            .formulaire
            .as_ref()
            .map(|f| f.modele.starts_with("/factures nouveau"))
            .unwrap_or(false));
    }

    #[test]
    fn les_fleches_changent_d_onglet_et_q_quitte() {
        let mut app = App::memoire();
        let mut vue = Vue::nouvelle();
        assert!(vue.touche(&mut app, KeyEvent::from(KeyCode::Right)));
        assert_eq!(vue.onglet, 1);
        assert!(vue.touche(&mut app, KeyEvent::from(KeyCode::Left)));
        assert_eq!(vue.onglet, 0);
        vue.touche(&mut app, KeyEvent::from(KeyCode::Left));
        assert_eq!(vue.onglet, ONGLETS.len() - 1);
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('q')));
        assert!(vue.quitter);
    }

    #[test]
    fn un_ecran_se_lit_sans_polluer_l_audit() {
        let mut app = App::memoire();
        app.executer("/clients nouveau --nom \"Zébulon\" --type particulier", &mut Vec::new())
            .unwrap();
        let avant = app.store.audit_complet().len();
        let mut vue = Vue::nouvelle();
        vue.onglet = 1;
        vue.rafraichir(&mut app);
        assert!(vue.lignes.iter().any(|l| l.contains("Zébulon")), "{:?}", vue.lignes);
        let apres = app.store.audit_complet().len();
        assert_eq!(avant, apres, "la navigation ne doit rien journaliser");
    }

    #[test]
    fn l_onglet_compta_affiche_le_tableau_financier() {
        let mut app = App::memoire();
        let mut vue = Vue::nouvelle();
        vue.onglet = 6; // Compta
        vue.rafraichir(&mut app);
        let texte = vue.lignes.join("\n");
        assert!(texte.contains("Tableau de bord finance"), "{texte}");
        assert!(!texte.contains("usage"), "{texte}");
    }

    #[test]
    fn le_tableau_de_bord_s_affiche_en_memoire() {
        let mut app = App::memoire();
        let mut vue = Vue::nouvelle();
        vue.rafraichir(&mut app);
        let contenu = vue.lignes.join("\n");
        assert!(contenu.contains("tableau de bord"), "{contenu}");
    }

    #[test]
    fn la_saisie_modifie_le_champ_actif() {
        let mut form = Formulaire::recherche();
        form.saisir('d');
        form.saisir('u');
        form.effacer();
        assert_eq!(form.champs[0].valeur, "d");
        form.avancer();
        assert_eq!(form.focus, 0); // un seul champ : retour au même
    }
}
