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
//! `←→` onglets · `↑↓` défilement · `n` nouveau · `c` statut prospect↔client
//! · `o` note · `p` paiement · `s` recherche · `e` société · `u` session
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
pub const RACCOURCIS: &str = "←→ onglets · ↑↓ défiler · n nouveau · c statut · o note \
· p paiement · s recherche · e société · u session · r rafraîchir · ? aide · q quitter";

// ------------------------------------------------------------------ Formulaire

/// Un champ de saisie. `cle` sert d'identifiant dans le modèle de commande.
#[derive(Debug, Clone)]
pub struct Champ {
    pub cle: &'static str,
    pub libelle: &'static str,
    pub valeur: String,
    pub obligatoire: bool,
    pub indice: &'static str,
    /// Bascule graphique (interrupteur) : les deux positions possibles, dans
    /// l'ordre (« éteinte », « allumée »). `None` = champ texte.
    pub bascule: Option<(&'static str, &'static str)>,
    /// Saisie masquée (mot de passe, code) : la valeur réelle n'est **jamais**
    /// affichée à l'écran, seulement des points.
    pub secret: bool,
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
            bascule: None,
            secret: false,
        }
    }

    /// Champ masqué : la frappe n'apparaît qu'en points (mot de passe, code).
    pub fn secret(
        cle: &'static str,
        libelle: &'static str,
        obligatoire: bool,
        indice: &'static str,
    ) -> Self {
        Self {
            secret: true,
            ..Self::new(cle, libelle, obligatoire, indice)
        }
    }

    /// Ce que l'écran a le droit de montrer de la valeur.
    pub fn affichee(&self) -> String {
        if self.secret {
            "•".repeat(self.valeur.chars().count())
        } else {
            self.valeur.clone()
        }
    }

    /// Interrupteur à deux positions (Espace pour basculer) : plus simple qu'un
    /// champ texte pour un choix binaire comme prospect/client.
    pub fn bascule(
        cle: &'static str,
        libelle: &'static str,
        positions: (&'static str, &'static str),
        indice: &'static str,
    ) -> Self {
        Self {
            bascule: Some(positions),
            valeur: positions.0.to_string(),
            ..Self::new(cle, libelle, false, indice)
        }
    }

    pub fn est_bascule(&self) -> bool {
        self.bascule.is_some()
    }

    /// Bascule sur l'autre position (sans effet sur un champ texte).
    pub fn basculer(&mut self) {
        if let Some((eteint, allume)) = self.bascule {
            self.valeur = if self.valeur == eteint {
                allume.to_string()
            } else {
                eteint.to_string()
            };
        }
    }

    /// Rendu de l'interrupteur : `[●] client   [○] prospect`.
    ///
    /// L'ordre des positions est fixe : seule la marque se déplace, l'œil ne
    /// « saute » pas d'un côté à l'autre quand on bascule.
    pub fn affichage_bascule(&self) -> Option<String> {
        let (premier, second) = self.bascule?;
        let marque = |position: &str| {
            if self.valeur == position {
                '●'
            } else {
                '○'
            }
        };
        Some(format!(
            "[{}] {premier}   [{}] {second}",
            marque(premier),
            marque(second)
        ))
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
    /// Le statut est un **interrupteur** : il démarre sur « client », la barre
    /// d'espace le passe sur « prospect » (piste commerciale).
    pub fn client() -> Self {
        let mut form = Self::nouveau(
            "Nouveau client / prospect",
            "/clients nouveau --nom \"{nom}\" --type {type} --statut {statut} \
             --email \"{email}\" --ville \"{ville}\"",
            vec![
                Champ::new("nom", "Nom", true, "raison sociale ou nom+prénom"),
                Champ::new("type", "Type", true, "particulier | professionnel | association"),
                Champ::bascule(
                    "statut",
                    "Statut (Espace pour basculer)",
                    ("client", "prospect"),
                    "interrupteur : client = relation établie, prospect = piste",
                ),
                Champ::new("email", "E-mail", false, "facultatif, sert à l'envoi"),
                Champ::new("ville", "Ville", false, "facultatif"),
            ],
        );
        form.champs[2].valeur = "client".to_string();
        form
    }

    /// Bascule prospect ↔ client d'une fiche existante (touche `c` de l'onglet
    /// Clients) : on saisit le nom, puis l'interrupteur dit où l'on va.
    pub fn statut_client() -> Self {
        Self::nouveau(
            "Statut de la fiche",
            "/clients \"{nom}\" statut {statut}",
            vec![
                Champ::new("nom", "Fiche", true, "nom, e-mail ou n° de client"),
                Champ::bascule(
                    "statut",
                    "Statut (Espace pour basculer)",
                    ("client", "prospect"),
                    "client = facturation ouverte · prospect = devis seulement",
                ),
            ],
        )
    }

    /// Connexion à un compte (sélecteur de session, touche `u`) : le mot de
    /// passe est **masqué**, le code 2FA facultatif (SEC-02 / SEC-03).
    pub fn connexion() -> Self {
        Self::nouveau(
            "Session : connexion",
            "/auth connexion \"{compte}\" --mot-de-passe \"{mot_de_passe}\" --totp \"{totp}\"",
            vec![
                Champ::new("compte", "Compte", true, "nom d'utilisateur"),
                Champ::secret(
                    "mot_de_passe",
                    "Mot de passe (masqué)",
                    true,
                    "12 caractères minimum — jamais affiché",
                ),
                Champ::new(
                    "totp",
                    "Code 2FA",
                    false,
                    "facultatif : 6 chiffres ou code reçu par e-mail",
                ),
            ],
        )
    }

    /// Nouvelle note sur une fiche (touche `o` de l'onglet Clients).
    pub fn note_client() -> Self {
        Self::nouveau(
            "Nouvelle note",
            "/clients \"{nom}\" note \"{texte}\"",
            vec![
                Champ::new("nom", "Fiche", true, "nom, e-mail ou n° de client"),
                Champ::new("texte", "Note", true, "appel, relance, information à garder"),
            ],
        )
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
            if champ.est_bascule() {
                // Sur un interrupteur, seule la barre d'espace agit : on ne
                // tape pas une valeur binaire au clavier.
                if c == ' ' {
                    champ.basculer();
                }
                return;
            }
            champ.valeur.push(c);
        }
    }

    pub fn effacer(&mut self) {
        if let Some(champ) = self.champs.get_mut(self.focus) {
            if champ.est_bascule() {
                return;
            }
            champ.valeur.pop();
        }
    }
}

// ------------------------------------------------------------------ Sélecteur

/// Une ligne du sélecteur : une société, un compte…
#[derive(Debug, Clone)]
pub struct OptionSelecteur {
    pub libelle: String,
    pub detail: String,
    /// Ligne de l'élément courant (marquée `▸`).
    pub courant: bool,
    /// Commande exécutée immédiatement quand on valide (bascule directe).
    pub commande: Option<String>,
    /// Compte à connecter : valider ouvre le formulaire de mot de passe.
    pub compte: Option<String>,
}

/// Sélecteur plein écran : une liste où l'on se déplace et où **Entrée** agit.
///
/// Deux usages, les deux questions du quotidien : « sur quelle société je
/// travaille ? » (`e`) et « sous quelle session ? » (`u`). L'élément courant
/// est marqué `▸`, la ligne choisie `❯` — le reste n'est qu'une commande du
/// CLI, donc aucun comportement propre à la TUI.
#[derive(Debug, Clone)]
pub struct Selecteur {
    pub titre: &'static str,
    pub invite: &'static str,
    pub options: Vec<OptionSelecteur>,
    pub index: usize,
}

impl Selecteur {
    /// Sélecteur des sociétés : une ligne par société, la courante marquée.
    pub fn societes(app: &mut App) -> Self {
        let courante = app.store.societe().map(|s| s.id).unwrap_or(0);
        let options: Vec<OptionSelecteur> = app
            .store
            .societes()
            .into_iter()
            .map(|s| OptionSelecteur {
                libelle: s.nom.clone(),
                detail: format!("{} · devise {}", s.regime.libelle(), s.devise),
                courant: s.id == courante,
                commande: Some(format!("/societes basculer \"{}\"", s.nom)),
                compte: None,
            })
            .collect();
        let index = options.iter().position(|o| o.courant).unwrap_or(0);
        Self {
            titre: "Sociétés — sélecteur",
            invite: "↑↓ choisir · Entrée basculer · Échap annuler",
            options,
            index,
        }
    }

    /// Sélecteur des sessions : les comptes déclarés + « session locale ».
    pub fn sessions(app: &mut App) -> Self {
        let session = app.store.session();
        let mut options: Vec<OptionSelecteur> = Vec::new();
        if session != "local" {
            options.push(OptionSelecteur {
                libelle: "Session locale".to_string(),
                detail: format!("déconnecter {session}"),
                courant: false,
                commande: Some("/auth deconnexion".to_string()),
                compte: None,
            });
        }
        for utilisateur in app.store.utilisateurs() {
            let actif = utilisateur.actif;
            options.push(OptionSelecteur {
                libelle: utilisateur.nom_utilisateur.clone(),
                detail: format!(
                    "{} · {}{}",
                    utilisateur.role.libelle(),
                    if utilisateur.deux_facteurs_actif() {
                        "2FA activée"
                    } else {
                        "sans 2FA"
                    },
                    if actif { "" } else { " · désactivé" }
                ),
                courant: utilisateur.nom_utilisateur == session,
                commande: None,
                compte: if actif {
                    Some(utilisateur.nom_utilisateur)
                } else {
                    None
                },
            });
        }
        Self {
            titre: "Sessions — sélecteur",
            invite: "↑↓ choisir · Entrée se connecter · Échap annuler",
            options,
            index: 0,
        }
    }

    /// Déplace la ligne choisie (avec rebouclage : on ne se perd jamais).
    pub fn deplacer(&mut self, pas: i32) {
        if self.options.is_empty() {
            return;
        }
        let n = self.options.len() as i32;
        self.index = ((self.index as i32 + pas).rem_euclid(n)) as usize;
    }

    pub fn courante(&self) -> Option<&OptionSelecteur> {
        self.options.get(self.index)
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
    pub selecteur: Option<Selecteur>,
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
            selecteur: None,
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

    /// Exécute une commande du CLI et garde la **première** ligne de la réponse
    /// comme message d'écran (c'est ce que le pied de page affiche).
    fn executer_et_recharger(&mut self, app: &mut App, commande: &str) -> bool {
        let mut tampon: Vec<u8> = Vec::new();
        match app.executer(commande, &mut tampon) {
            Ok(_) => {
                self.message = sans_ansi(&String::from_utf8_lossy(&tampon))
                    .lines()
                    .next()
                    .unwrap_or("fait")
                    .to_string();
                self.succes = true;
                self.recherhe = None;
                true
            }
            Err(e) => {
                self.message = e;
                self.succes = false;
                false
            }
        }
    }

    /// Applique une touche. Renvoie `true` si un rafraîchissement est utile.
    ///
    /// Trois couches, dans cet ordre : le **sélecteur** (société / session), le
    /// **formulaire** de saisie, puis la navigation. Un sélecteur ouvert capte
    /// tout : on ne peut pas changer d'onglet au milieu d'un choix.
    pub fn touche(&mut self, app: &mut App, touche: KeyEvent) -> bool {
        if touche.kind != KeyEventKind::Press {
            return false;
        }
        // 0. Sélecteur (société, session) : il capte tout tant qu'il est ouvert.
        if let Some(mut sel) = self.selecteur.take() {
            match touche.code {
                KeyCode::Esc => {
                    self.message = "Sélection annulée".to_string();
                    self.succes = false;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    sel.deplacer(-1);
                    self.selecteur = Some(sel);
                }
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                    sel.deplacer(1);
                    self.selecteur = Some(sel);
                }
                KeyCode::Home => {
                    sel.index = 0;
                    self.selecteur = Some(sel);
                }
                KeyCode::End => {
                    sel.index = sel.options.len().saturating_sub(1);
                    self.selecteur = Some(sel);
                }
                KeyCode::Enter => match sel.courante().cloned() {
                    None => {
                        self.message = "Aucun choix disponible — créez-le d'abord".to_string();
                        self.succes = false;
                    }
                    Some(option) => match (option.commande, option.compte) {
                        (Some(commande), _) => return self.executer_et_recharger(app, &commande),
                        // Un compte : on demande le mot de passe (masqué).
                        (None, Some(compte)) => {
                            let mut form = Formulaire::connexion();
                            form.champs[0].valeur = compte;
                            form.focus = 1;
                            self.formulaire = Some(form);
                        }
                        (None, None) => {
                            self.message = format!("{} est désactivé", option.libelle);
                            self.succes = false;
                        }
                    },
                },
                _ => self.selecteur = Some(sel),
            }
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
                        if self.executer_et_recharger(app, &commande) {
                            return true;
                        }
                        // Échec : on rend la main au formulaire avec le message
                        // exact du CLI (aucune règle métier n'est recopiée ici).
                        form.erreur = Some(self.message.clone());
                        self.formulaire = Some(form);
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
            // `e` : sélecteur de société (multi-sociétés, EF-SOC-01).
            KeyCode::Char('e') => {
                self.selecteur = Some(Selecteur::societes(app));
            }
            // `u` : sélecteur de session (les comptes, SEC-01).
            KeyCode::Char('u') => {
                self.selecteur = Some(Selecteur::sessions(app));
            }
            // `c` : interrupteur prospect ↔ client (CDC Module A).
            KeyCode::Char('c') => {
                self.formulaire = Some(Formulaire::statut_client());
            }
            // `o` : nouvelle note datée sur une fiche.
            KeyCode::Char('o') => {
                self.formulaire = Some(Formulaire::note_client());
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

fn dessiner(
    frame: &mut Frame,
    vue: &Vue,
    societe: &str,
    regime: &str,
    devise: &str,
    session: &str,
) {
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
            format!("· {regime} · {devise} · session {session} · {} ", vue.chemin().trim()),
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
    if let Some(selecteur) = &vue.selecteur {
        dessiner_selecteur(frame, selecteur);
    }
    if let Some(form) = &vue.formulaire {
        dessiner_formulaire(frame, form);
    }
}

/// Rend le sélecteur : `▸` = élément courant, `❯` = ligne choisie.
fn dessiner_selecteur(frame: &mut Frame, sel: &Selecteur) {
    let hauteur = (sel.options.len() as u16 + 6).min(frame.area().height);
    let zone = centre_hauteur(frame.area(), 72, hauteur);
    frame.render_widget(Clear, zone);

    let mut lignes: Vec<TxtLine> = Vec::new();
    if sel.options.is_empty() {
        lignes.push(TxtLine::from(Span::styled(
            "  (aucun élément — créez-le avec une commande du CLI)",
            Style::default().fg(rgb(theme::ORANGE_RGB)),
        )));
    }
    for (index, option) in sel.options.iter().enumerate() {
        let choisi = index == sel.index;
        let style = if choisi {
            style_onglet_actif()
        } else {
            Style::default().fg(rgb(theme::BLANC_RGB))
        };
        lignes.push(TxtLine::from(Span::styled(
            format!(
                " {} {} {:<26} {}",
                if option.courant { "▸" } else { " " },
                if choisi { "❯" } else { " " },
                option.libelle,
                option.detail
            ),
            style,
        )));
    }
    lignes.push(TxtLine::from(""));
    lignes.push(TxtLine::from(Span::styled(
        sel.invite,
        style_onglet_inactif(),
    )));

    let bloc = Block::default()
        .borders(Borders::ALL)
        .border_style(style_violet())
        .title(Span::styled(format!(" {} ", sel.titre), style_titre()));
    frame.render_widget(
        Paragraph::new(lignes)
            .block(bloc)
            .wrap(Wrap { trim: false }),
        zone,
    );
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
        "Prospect ↔ client : touche c, puis Espace sur l'interrupteur, Entrée pour valider.",
    ));
    lignes.push(TxtLine::from(
        "Notes : touche o — une note datée est conservée avec la fiche (journal).",
    ));
    lignes.push(TxtLine::from(
        "Société : touche e — sélecteur, Entrée bascule (données séparées par société).",
    ));
    lignes.push(TxtLine::from(
        "Session : touche u — sélecteur des comptes, mot de passe masqué (12 caractères min.).",
    ));
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
        match champ.affichage_bascule() {
            Some(rendu) => lignes.push(TxtLine::from(Span::styled(format!("  {rendu}"), style))),
            None => lignes.push(TxtLine::from(Span::styled(
                format!("  {}▏", champ.affichee()),
                style,
            ))),
        }
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
        "Entrée valider · Tab champ suivant · Espace bascule · Échap annuler",
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
        let session = app.store.session();
        terminal
            .draw(|frame| dessiner(frame, &vue, &nom, &regime, &devise, &session))
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
    fn l_interrupteur_de_statut_bascule_et_alimente_la_commande() {
        let mut form = Formulaire::client();
        let statut = &form.champs[2];
        assert!(statut.est_bascule());
        assert_eq!(statut.valeur, "client", "client par défaut");
        // Rendu lisible : la position active est marquée.
        let rendu = statut.affichage_bascule().unwrap();
        assert!(rendu.contains("[●] client"), "{rendu}");
        assert!(rendu.contains("[○] prospect"), "{rendu}");
        // Espace bascule, les autres touches ne touchent pas à l'interrupteur.
        form.focus = 2;
        form.saisir('x');
        assert_eq!(form.champs[2].valeur, "client");
        form.saisir(' ');
        assert_eq!(form.champs[2].valeur, "prospect");
        assert!(form.champs[2].affichage_bascule().unwrap().contains("[●] prospect"));
        form.saisir(' ');
        assert_eq!(form.champs[2].valeur, "client");
        form.saisir(' ');
        // Retour arrière ne casse pas un interrupteur.
        form.effacer();
        assert_eq!(form.champs[2].valeur, "prospect");
        form.champs[0].valeur = "Piste SARL".into();
        form.champs[1].valeur = "professionnel".into();
        let commande = form.commande().unwrap();
        assert!(commande.ends_with("--statut prospect"), "{commande}");
    }

    #[test]
    fn la_touche_c_ouvre_le_switch_de_statut_et_valide() {
        let mut app = App::memoire();
        app.executer(
            "/clients nouveau --nom \"Piste SARL\" --type pro --prospect",
            &mut Vec::new(),
        )
        .unwrap();
        let mut vue = Vue::nouvelle();
        vue.onglet = 1; // Clients
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('c')));
        let form = vue.formulaire.as_ref().expect("formulaire statut");
        assert_eq!(form.champs[1].valeur, "client", "l'interrupteur propose client");
        // Saisie du nom, puis bascule déjà sur « client » : Entrée convertit.
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('P')));
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('i')));
        vue.touche(&mut app, KeyEvent::from(KeyCode::Enter));
        assert!(vue.formulaire.is_none(), "formulaire refermé");
        assert!(vue.message.contains("statut"), "{}", vue.message);
        assert!(!vue.message.contains('✗'), "{}", vue.message);
        assert!(app.store.clients().iter().all(|c| !c.est_prospect()));
    }

    #[test]
    fn la_touche_o_ajoute_une_note_a_une_fiche() {
        let mut app = App::memoire();
        app.executer("/clients nouveau --nom \"Dupont\" --type pro", &mut Vec::new())
            .unwrap();
        let mut vue = Vue::nouvelle();
        vue.onglet = 1;
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('o')));
        assert!(vue.formulaire.is_some());
        for c in "Dupont".chars() {
            vue.touche(&mut app, KeyEvent::from(KeyCode::Char(c)));
        }
        vue.touche(&mut app, KeyEvent::from(KeyCode::Tab));
        for c in "Rappeler lundi".chars() {
            vue.touche(&mut app, KeyEvent::from(KeyCode::Char(c)));
        }
        vue.touche(&mut app, KeyEvent::from(KeyCode::Enter));
        assert!(vue.formulaire.is_none(), "{:?}", vue.formulaire);
        let client = app.store.clients()[0].clone();
        let notes = app.store.notes_client(client.id);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].texte, "Rappeler lundi");
        assert!(vue.message.contains("Note"), "{}", vue.message);
    }

    #[test]
    fn le_selecteur_de_societes_bascule_au_clavier() {
        let mut app = App::memoire();
        let nom_depart = app.store.societe().unwrap().nom;
        lancer_cli(&mut app, "/societes nouvelle --nom \"Atelier Bis\" --regime reel");
        let mut vue = Vue::nouvelle();
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('e')));
        let sel = vue.selecteur.as_ref().expect("sélecteur de sociétés");
        assert_eq!(sel.options.len(), 2, "une ligne par société");
        assert!(
            sel.options[sel.index].courant,
            "la ligne choisie au départ est la société courante"
        );
        assert!(sel.options.iter().any(|o| o.libelle == "Atelier Bis"));
        // ↓ puis Entrée : on bascule sur l'autre société, en une touche.
        vue.touche(&mut app, KeyEvent::from(KeyCode::Down));
        vue.touche(&mut app, KeyEvent::from(KeyCode::Enter));
        assert!(vue.selecteur.is_none(), "le sélecteur se referme");
        assert!(vue.message.contains("Société courante"), "{}", vue.message);
        assert_eq!(app.store.societe().unwrap().nom, nom_depart);
        // Échap referme sans rien changer.
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('e')));
        vue.touche(&mut app, KeyEvent::from(KeyCode::Esc));
        assert!(vue.selecteur.is_none());
        assert!(vue.message.contains("annulée"), "{}", vue.message);
        assert_eq!(app.store.societe().unwrap().nom, nom_depart);
    }

    #[test]
    fn le_selecteur_de_sessions_ouvre_une_connexion_masquee() {
        let mut app = App::memoire();
        // Le premier compte doit être administrateur (règle du magasin).
        lancer_cli(
            &mut app,
            "/utilisateurs nouveau admin --mot-de-passe \"MotDePasse123!\" --role admin",
        );
        lancer_cli(
            &mut app,
            "/utilisateurs nouveau jojo --mot-de-passe \"MotDePasse123!\" --role utilisateur",
        );
        let mut vue = Vue::nouvelle();
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('u')));
        let sel = vue.selecteur.as_ref().expect("sélecteur de sessions");
        assert_eq!(sel.options.len(), 2, "une ligne par compte");
        let position = sel
            .options
            .iter()
            .position(|o| o.libelle == "jojo")
            .expect("le compte jojo est proposé");
        assert!(
            sel.options[position].detail.contains("utilisateur"),
            "{:?}",
            sel.options[position].detail
        );
        for _ in 0..position {
            vue.touche(&mut app, KeyEvent::from(KeyCode::Down));
        }
        // Entrée : le mot de passe est demandé, jamais affiché.
        vue.touche(&mut app, KeyEvent::from(KeyCode::Enter));
        let form = vue.formulaire.as_ref().expect("formulaire de connexion");
        assert_eq!(form.champs[0].valeur, "jojo", "compte pré-rempli");
        assert!(form.champs[1].secret);
        for c in "MotDePasse123!".chars() {
            vue.touche(&mut app, KeyEvent::from(KeyCode::Char(c)));
        }
        let form = vue.formulaire.as_ref().unwrap();
        assert_eq!(form.champs[1].valeur, "MotDePasse123!");
        assert_eq!(form.champs[1].affichee(), "•".repeat(14));
        assert!(!form.champs[1].affichee().contains("MotDePasse"));
        vue.touche(&mut app, KeyEvent::from(KeyCode::Enter));
        assert!(vue.formulaire.is_none(), "{:?}", vue.formulaire);
        assert_eq!(app.store.session(), "jojo");
        assert!(vue.message.contains("Connecté"), "{}", vue.message);
        // Le sélecteur propose alors la session locale (déconnexion) et marque
        // le compte connecté.
        vue.touche(&mut app, KeyEvent::from(KeyCode::Char('u')));
        let sel = vue.selecteur.as_ref().unwrap();
        assert_eq!(sel.options[0].libelle, "Session locale");
        let jojo = sel
            .options
            .iter()
            .find(|o| o.libelle == "jojo")
            .expect("compte jojo");
        assert!(jojo.courant, "le compte connecté est marqué");
        vue.touche(&mut app, KeyEvent::from(KeyCode::Enter));
        assert_eq!(app.store.session(), "local");
    }

    #[test]
    fn un_selecteur_sans_choix_le_dit_au_lieu_de_planter() {
        let mut app = App::memoire();
        let mut vue = Vue::nouvelle();
        vue.selecteur = Some(Selecteur {
            titre: "Vide",
            invite: "rien à choisir",
            options: Vec::new(),
            index: 0,
        });
        vue.touche(&mut app, KeyEvent::from(KeyCode::Enter));
        assert!(vue.selecteur.is_none());
        assert!(vue.message.contains("Aucun choix"), "{}", vue.message);
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
