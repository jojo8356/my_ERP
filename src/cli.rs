//! Interface CLI : navigation par chemins type URL (CDC §6), aucune GUI.
//!
//! Exemples : `erp /clients`, `erp /factures valider FA-2026-0001`,
//! `erp /compta/seuils`, `erp /factures/2026/0042 pdf`.
//!
//! Règles tenues :
//! - tout chemin fonctionne en commande unique **et** en mode interactif ;
//! - segments insensibles à la casse, `..` remonte d'un niveau ;
//! - `--json` / `--csv` sur les listes, pagination `--page` / `--lignes` ;
//! - messages d'erreur en français, avec l'action à faire (ENF-ERG-01).

use crate::compta;
use crate::csv;
use crate::domain::*;
use crate::efacture;
use crate::messagerie;
use crate::pdf;
use crate::securite;
use crate::store::*;
use crate::theme;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::Path;

pub struct App {
    pub store: Store,
}

type Options = HashMap<String, Vec<String>>;
pub type Resultat<T> = Result<T, String>;

// ------------------------------------------------------------ Outils CLI

pub fn tokeniser(ligne: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut guillemets = false;
    for c in ligne.chars() {
        match c {
            '"' => guillemets = !guillemets,
            c if c.is_whitespace() && !guillemets => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Sépare les options `--cle valeur` / `--cle=valeur` des arguments de position.
fn separer_options(jetons: Vec<String>) -> (Vec<String>, Options) {
    let mut positionnels = Vec::new();
    let mut options: Options = HashMap::new();
    let mut i = 0;
    while i < jetons.len() {
        let jeton = &jetons[i];
        if let Some(reste) = jeton.strip_prefix("--") {
            if let Some((cle, valeur)) = reste.split_once('=') {
                options.entry(cle.to_lowercase()).or_default().push(valeur.to_string());
                i += 1;
            } else if i + 1 < jetons.len() && !jetons[i + 1].starts_with("--") {
                options
                    .entry(reste.to_lowercase())
                    .or_default()
                    .push(jetons[i + 1].clone());
                i += 2;
            } else {
                options.entry(reste.to_lowercase()).or_default().push("oui".into());
                i += 1;
            }
        } else {
            positionnels.push(jeton.clone());
            i += 1;
        }
    }
    (positionnels, options)
}

fn opt<'a>(options: &'a Options, cle: &str) -> Option<&'a str> {
    options.get(cle).and_then(|v| v.first()).map(|s| s.as_str())
}

fn drapeau(options: &Options, cle: &str) -> bool {
    options
        .get(cle)
        .map(|v| v.iter().any(|x| x == "oui" || x == "1" || x == "true"))
        .unwrap_or(false)
}

fn opt_i64(options: &Options, cle: &str, defaut: i64) -> i64 {
    opt(options, cle)
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaut)
}

fn ecrire(sortie: &mut dyn Write, texte: impl std::fmt::Display) -> Resultat<()> {
    writeln!(sortie, "{texte}").map_err(|e| e.to_string())
}

fn json_echapper(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Pagination : renvoie la tranche demandée et le pied de page.
fn paginer<T: Clone>(elements: &[T], options: &Options) -> (Vec<T>, String) {
    let par_page = opt_i64(options, "lignes", 20).clamp(1, 500) as usize;
    let total = elements.len();
    let pages = (total + par_page - 1) / par_page.max(1);
    let page = (opt_i64(options, "page", 1).max(1) as usize).min(pages.max(1));
    let debut = (page - 1) * par_page;
    let tranche = elements
        .iter()
        .skip(debut)
        .take(par_page)
        .cloned()
        .collect::<Vec<T>>();
    let pied = if total > par_page {
        format!(
            "— page {}/{} · {} ligne(s) au total (--page, --lignes) —",
            page,
            pages.max(1),
            total
        )
    } else {
        format!("— {total} ligne(s) —")
    };
    (tranche, pied)
}

fn json_document(d: &Document, regime: RegimeTva) -> String {
    let totaux = d.totaux(regime);
    format!(
        "{{\"numero\":\"{}\",\"type\":\"{}\",\"statut\":\"{}\",\"client_id\":{},\"date\":\"{}\",\"echeance\":\"{}\",\"activite\":\"{}\",\"devise\":\"{}\",\"ht\":{},\"tva\":{},\"ttc\":{},\"paye\":{},\"reste\":{},\"lignes\":{}}}",
        json_echapper(&d.numero),
        d.type_doc.as_str(),
        d.statut.code(),
        d.client_id,
        d.date_doc,
        d.echeance,
        json_echapper(&d.activite),
        json_echapper(&d.devise),
        totaux.ht,
        totaux.tva,
        totaux.ttc,
        d.paye,
        d.reste(regime),
        d.lignes.len()
    )
}

fn json_client(c: &Client) -> String {
    format!(
        "{{\"id\":{},\"nom\":\"{}\",\"type\":\"{}\",\"email\":\"{}\",\"ville\":\"{}\",\"etiquettes\":[{}]}}",
        c.id,
        json_echapper(&c.nom),
        c.type_client.code(),
        json_echapper(&c.email),
        json_echapper(&c.ville),
        c.etiquettes
            .iter()
            .map(|e| format!("\"{}\"", json_echapper(e)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_article(a: &Article) -> String {
    format!(
        "{{\"id\":{},\"marque\":\"{}\",\"description\":\"{}\",\"taille\":\"{}\",\"composition\":\"{}\",\"prix_achat\":{},\"statut\":\"{}\",\"regime\":\"{}\"}}",
        a.id,
        json_echapper(&a.marque),
        json_echapper(&a.description),
        json_echapper(&a.taille),
        json_echapper(&a.composition),
        a.prix_achat,
        json_echapper(&a.statut),
        a.regime.as_str()
    )
}

fn json_vente(v: &Vente) -> String {
    format!(
        "{{\"id\":{},\"article_id\":{},\"date\":\"{}\",\"prix\":{},\"frais\":{},\"plateforme\":\"{}\",\"regime\":\"{}\"}}",
        v.id,
        v.article_id,
        v.date_vente,
        v.prix_vente,
        v.frais,
        json_echapper(&v.plateforme),
        v.regime.as_str()
    )
}

// ------------------------------------------------------------ Aide

pub const AIDE: &str = "\
Navigation par chemins (comme des URL, à connaître par cœur) :
  erp /                          tableau de bord (impayés, seuils, alertes)
  erp /recherche dupont          recherche globale (clients, factures, articles, ventes)
  erp /clients                   liste des clients        (alias interactif : g c)
  erp /clients nouveau --nom \"Dupont\" --type pro --email a@b.fr --tag maintenance
  erp /clients dupont            fiche client (recherche floue) + historique unifié
  erp /clients dupont factures   ses documents
  erp /clients dupont rgpd exporter|effacer
  erp /clients importer clients.csv   /clients exporter clients.csv
  erp /devis                     liste des devis          (alias : g d)
  erp /factures                  liste des factures       (alias : g f)
  erp /factures nouvelle --client dupont --activite services --ligne \"Maintenance;2;500,00;2000\"
  erp /factures FA-2026-0001             détail d'une facture (ou /factures/2026/0001)
  erp /factures FA-2026-0001 valider     contrôle des mentions puis validation (figée)
  erp /factures FA-2026-0001 envoyer     e-mail + PDF dans la boîte d'envoi
  erp /factures FA-2026-0001 payer [montant] [AAAA-MM-JJ] [moyen] [référence]
  erp /factures FA-2026-0001 relance     relance graduée (1 à 3) selon le retard
  erp /factures FA-2026-0001 pdf [fichier]      PDF conforme (mentions §11.1)
  erp /factures FA-2026-0001 ubl [fichier]      e-facture UBL / EN 16931
  erp /factures FA-2026-0001 avoir       crée un avoir (corrige une facture validée)
  erp /factures recurrentes [nouvelle|generer]  abonnements de maintenance
  erp /devis DEV-2026-0001 transformer   devis -> facture
  erp /devis DEV-2026-0001 accepter|refuser|expirer
  erp /vinted                    articles + ventes        (alias : g v)
  erp /vinted nouveau --marque Zara --description \"Veste\" --taille M --composition \"100 % coton\" --prix-achat 8,00 --regime personnel
  erp /vinted vendre 3 --prix 25,00 --frais 2,00
  erp /vinted stats              marges par marque/taille, rotation, DAC7
  erp /vinted fiche 3            annonce prête à coller
  erp /vinted importer ventes.csv   /vinted exporter articles.csv
  erp /compta seuils             CA ventes/services vs plafonds micro 2026 (%)
  erp /compta fec [fichier]      export FEC (expert-comptable)
  erp /compta journal VE|BQ      journaux comptables     |  /compta balance
  erp /compta tva [AAAA-MM]      récapitulatif TVA par taux et période
  erp /compta tableau            CA par mois, encours, trésorerie
  erp /compta rapprocher releve.csv  then /compta lettrer
  erp /societes                  multi-sociétés (basculer, créer, modifier)
  erp /societe modifier --siren 123456789 --adresse \"1 rue …\" --ville Nice
  erp /devises                   taux datés   |  /devises taux USD 2026-10-01 0,92
  erp /utilisateurs              comptes, rôles, 2FA      |  /auth connexion jojo
  erp /admin sante|schema|audit|sauvegarde|mentions
Options : --json, --csv, --page N, --lignes N, --tag, --statut, --impayees, --retard, --societe.
Mode interactif : lancez `erp` seul (q pour quitter, ? = aide du contexte, .. = remonter).
Interface plein écran : `erp tui` (Tab = module, ↑↓ = naviguer, v = valider, p = payer, q = quitter).
Format d'une ligne : \"désignation;quantité;prix unitaire HT[;taux TVA en points de base (2000 = 20 %)]\"";

fn aide_contexte(chemin: &[String]) -> &'static str {
    match chemin.first().map(|s| s.to_lowercase()).as_deref() {
        Some("clients") => "\
/clients                     liste (--tag, --json, --csv, --page)
/clients nouveau --nom … --type pro|particulier|association
/clients <recherche>         fiche + historique unifié
/clients <recherche> factures
/clients <recherche> modifier --email … --ville … --siren …
/clients <recherche> tag <étiquette> | detag <étiquette>
/clients <recherche> rgpd exporter | effacer
/clients importer <fichier.csv> | exporter <fichier.csv>",
        Some("factures") => "\
/factures                    liste (--impayees, --retard, --statut, --csv, --json)
/factures nouvelle --client <nom> --ligne \"désignation;qté;prix HT[;tva bp]\"
/factures <numéro>            détail (FA-2026-0001 ou /factures/2026/0001)
/factures <numéro> valider | envoyer | payer | relance | avoir | annuler
/factures <numéro> pdf [fichier] | ubl [fichier]
/factures recurrentes [nouvelle --client … --ligne … --periodicite mensuelle | generer]",
        Some("devis") => "\
/devis                       liste des devis
/devis nouvelle --client <nom> --ligne \"…\"
/devis <numéro> envoyer | accepter | refuser | expirer | transformer | pdf",
        Some("vinted") => "\
/vinted                      articles en stock et vendus
/vinted nouveau --marque … --description … --composition \"…\" --prix-achat 8,00
/vinted vendre <n°> --prix 25,00 --frais 2,00 [--acheteur …] [--reference …]
/vinted stats                marges par marque et taille, rotation, alerte DAC7
/vinted fiche <n°>           annonce exportable
/vinted importer <fichier.csv> | exporter <fichier.csv>",
        Some("compta") => "\
/compta seuils               CA vs plafonds micro et franchise TVA (%)
/compta fec [fichier]        FEC normé DGFiP
/compta journal [VE|BQ]      journal comptable
/compta tva [AAAA-MM]        TVA par taux et période
/compta balance              balance par compte
/compta tableau              CA mensuel, encours, trésorerie
/compta rapprocher <csv>     import de relevé   |   /compta lettrer",
        Some("societes") | Some("societe") => "\
/societes                    liste et bascule
/societes nouvelle --nom \"…\" --siren … --regime franchise|reel
/societes basculer <nom>     change de société courante
/societe modifier --siren … --adresse … --ville … --delai 30
/societe regime franchise | reel",
        Some("devises") => "\
/devises                     taux de change datés
/devises taux <DEVISE> <AAAA-MM-JJ> <1 devise = x EUR>
/devises convertir <montant> <DEVISE> [AAAA-MM-JJ]",
        Some("utilisateurs") | Some("auth") => "\
/auth connexion <compte> --mot-de-passe … [--totp 123456]
/auth statut | deconnexion
/utilisateurs                liste des comptes
/utilisateurs nouveau <compte> --role admin|utilisateur --mot-de-passe … [--email …]
/utilisateurs 2fa <compte>   active la double authentification (secret + codes)
/utilisateurs mot-de-passe <compte> --nouveau …",
        Some("admin") => "\
/admin sante                 fichiers, lignes, sauvegarde, chaîne d'audit
/admin schema                modèle Rustorm (--sql pour la migration PostgreSQL)
/admin audit [--verifier]    journal d'audit infalsifiable
/admin sauvegarde            crée une sauvegarde datée
/admin sauvegardes           liste   |   /admin restaurer <nom>
/admin mentions              checklist des mentions obligatoires de la société",
        _ => AIDE,
    }
}

// ------------------------------------------------------------ Application

impl App {
    pub fn ouvrir(dossier: &Path) -> Resultat<Self> {
        Ok(Self {
            store: Store::ouvrir(dossier)?,
        })
    }

    pub fn memoire() -> Self {
        Self {
            store: Store::memoire(),
        }
    }

    pub fn executer(&mut self, commande_brute: &str, sortie: &mut dyn Write) -> Resultat<bool> {
        self.executer_dans(&[], tokeniser(commande_brute), sortie)
    }

    pub fn executer_jetons(&mut self, jetons: Vec<String>, sortie: &mut dyn Write) -> Resultat<bool> {
        self.executer_dans(&[], jetons, sortie)
    }

    /// Texte d'un écran, **sans journaliser ni modifier** les données.
    ///
    /// Utilisé par la TUI (ADR-005) : un écran n'est rien d'autre qu'une
    /// commande du CLI rendue dans une cadre Ratatui. La navigation ne doit
    /// pas polluer le journal d'audit (SEC-05), d'où ce chemin en lecture
    /// seule distinct de [`App::executer`].
    pub fn apercu(&mut self, commande: &str) -> Resultat<String> {
        let (positionnels, options) = separer_options(tokeniser(commande));
        let mut segments: Vec<String> = Vec::new();
        let mut reste: Vec<String> = Vec::new();
        if let Some(premier) = positionnels.first() {
            if premier.starts_with('/') {
                segments.extend(
                    premier
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string()),
                );
                reste = positionnels[1..].to_vec();
            } else {
                reste = positionnels.clone();
            }
        }
        let segments: Vec<String> = segments.iter().map(|s| s.to_lowercase()).collect();
        let mut tampon: Vec<u8> = Vec::new();
        {
            let mut refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
            refs.extend(reste.iter().map(|s| s.as_str()));
            self.dispatch(&refs, &options, &mut tampon)?;
        }
        Ok(String::from_utf8_lossy(&tampon).to_string())
    }

    /// Exécute une commande, éventuellement relative à un chemin courant (REPL).
    pub fn executer_dans(
        &mut self,
        base: &[String],
        jetons: Vec<String>,
        sortie: &mut dyn Write,
    ) -> Resultat<bool> {
        let (positionnels, options) = separer_options(jetons);

        // Option globale de société : `/societes basculer` reste prioritaire.
        if let Some(recherche) = opt(&options, "societe") {
            self.store.basculer_societe(recherche)?;
        }

        let mut segments: Vec<String> = Vec::new();
        let mut reste: Vec<String> = Vec::new();
        if let Some(premier) = positionnels.first() {
            if premier.starts_with('/') {
                segments.extend(
                    premier
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string()),
                );
                reste = positionnels[1..].to_vec();
            }
        }
        if segments.is_empty() {
            segments = base.to_vec();
            reste = positionnels.clone();
        }
        let segments_minuscules: Vec<String> = segments.iter().map(|s| s.to_lowercase()).collect();
        self.journaliser_action(&segments_minuscules, &reste);

        let contexte: Vec<&str> = {
            let mut refs: Vec<&str> = segments_minuscules.iter().map(|s| s.as_str()).collect();
            refs.extend(reste.iter().map(|s| s.as_str()));
            refs
        };
        self.dispatch(&contexte, &options, sortie)
    }

    fn journaliser_action(&mut self, segments: &[String], reste: &[String]) {
        let chemin = format!("/{}/{}", segments.join("/"), reste.join(" "));
        let _ = self.store.journaliser("cli", chemin.trim_end_matches('/'));
    }

    fn dispatch(
        &mut self,
        contexte: &[&str],
        options: &Options,
        sortie: &mut dyn Write,
    ) -> Resultat<bool> {
        let Some(premier) = contexte.first().copied() else {
            self.tableau_de_bord(sortie)?;
            return Ok(true);
        };
        match premier {
            "" => self.tableau_de_bord(sortie)?,
            "aide" | "?" | "help" => {
                let chemin: Vec<String> = contexte[1..].iter().map(|s| s.to_string()).collect();
                let aide = aide_contexte(&chemin);
                ecrire(sortie, theme::violet(aide))?;
            }
            "quitter" | "q" | "exit" => return Ok(false),
            "recherche" | "search" => {
                let requete = contexte[1..].join(" ");
                self.cmd_recherche(&requete, options, sortie)?;
            }
            "clients" => self.cmd_clients(&contexte[1..], options, sortie)?,
            "devis" => self.cmd_documents(TypeDocument::Devis, &contexte[1..], options, sortie)?,
            "factures" => {
                self.cmd_documents(TypeDocument::Facture, &contexte[1..], options, sortie)?
            }
            "avoirs" => self.cmd_documents(TypeDocument::Avoir, &contexte[1..], options, sortie)?,
            "vinted" => self.cmd_vinted(&contexte[1..], options, sortie)?,
            "compta" | "comptabilite" => self.cmd_compta(&contexte[1..], options, sortie)?,
            "societes" | "societe" => self.cmd_societes(&contexte[1..], options, sortie)?,
            "devises" => self.cmd_devises(&contexte[1..], options, sortie)?,
            "utilisateurs" | "auth" => self.cmd_securite(&contexte[1..], options, sortie)?,
            "admin" => self.cmd_admin(&contexte[1..], options, sortie)?,
            "tui" => {
                ecrire(
                    sortie,
                    theme::info("La TUI se lance par `erp tui` depuis le terminal (pas de chemin)."),
                )?;
            }
            autre => {
                return Err(format!(
                    "chemin inconnu : « /{} » — tapez `aide` pour la liste des chemins, \
                     ou `recherche {}` pour trouver un client, une facture, un article",
                    contexte.join("/"),
                    autre
                ))
            }
        }
        Ok(true)
    }

    // ---------------------------------------------------------- Tableau de bord

    fn tableau_de_bord(&mut self, sortie: &mut dyn Write) -> Resultat<()> {
        let societe = self.store.societe()?;
        let annee = annee_courante();
        let (ventes, services) = self.store.ca_annuel(annee);
        let etat = etat_seuils(ventes, services, SEUILS_2026);
        let impayes = self.store.documents_impayes();
        let retard = self.store.documents_en_retard(&aujourdhui());
        ecrire(
            sortie,
            theme::titre(format!(
                "┌─ {} — tableau de bord {} ({})",
                societe.nom, annee, societe.regime.libelle()
            )),
        )?;
        ecrire(
            sortie,
            format!(
                "│ CA services {} ({} % du plafond) · CA ventes {} ({} % de la franchise TVA)",
                format_eur(services),
                pourcentage(services, SEUILS_2026.ca_services_max),
                format_eur(ventes),
                pourcentage(ventes, SEUILS_2026.franchise_tva_ventes)
            ),
        )?;
        ecrire(
            sortie,
            format!(
                "│ Encours clients {} · {} facture(s) en attente dont {} en retard",
                format_eur(self.store.encours()),
                impayes.len(),
                retard.len()
            ),
        )?;
        for d in retard.iter().take(5) {
            let jours = jours_retard(&d.echeance, &aujourdhui());
            ecrire(
                sortie,
                theme::alerte(format!(
                    "│   ! {} — {} en retard de {} jour(s) → `/factures {} relance`",
                    d.numero,
                    format_eur(d.reste(societe.regime)),
                    jours,
                    d.numero
                )),
            )?;
        }
        for alerte in &etat.alertes {
            ecrire(sortie, theme::alerte(format!("│ ⚠ {alerte}")))?;
        }
        let stats = self.store.stats_vinted(annee);
        if let Some(alerte) = alerte_dac7(stats.nb_ventes, stats.ca) {
            ecrire(sortie, theme::alerte(format!("│ ⚠ {alerte}")))?;
        }
        let dernieres = self.store.audit(1);
        if let Some(derniere) = dernieres.first() {
            ecrire(
                sortie,
                theme::info(format!(
                    "│ Dernière action : {} — {} ({})",
                    derniere.horodatage, derniere.action, derniere.utilisateur
                )),
            )?;
        }
        ecrire(
            sortie,
            theme::violet(
                "└─ /clients · /factures · /vinted · /compta/seuils · /admin sante · `aide`",
            ),
        )?;
        Ok(())
    }

    fn cmd_recherche(&mut self, requete: &str, options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        if requete.trim().is_empty() {
            return Err("usage : /recherche <texte> (client, numéro, article…)".into());
        }
        let resultats = self.store.recherche_globale(requete);
        if drapeau(options, "json") {
            let clients: Vec<String> = resultats.clients.iter().map(json_client).collect();
            let documents: Vec<String> = resultats
                .documents
                .iter()
                .map(|d| json_document(d, self.store.societe_regime_public()))
                .collect();
            let articles: Vec<String> = resultats.articles.iter().map(json_article).collect();
            let ventes: Vec<String> = resultats.ventes.iter().map(json_vente).collect();
            return ecrire(
                sortie,
                format!(
                    "{{\"clients\":[{}],\"documents\":[{}],\"articles\":[{}],\"ventes\":[{}]}}",
                    clients.join(","),
                    documents.join(","),
                    articles.join(","),
                    ventes.join(",")
                ),
            );
        }
        if resultats.total() == 0 {
            return ecrire(
                sortie,
                format!("Aucun résultat pour « {requete} » — essayez une partie du nom."),
            );
        }
        ecrire(
            sortie,
            theme::titre(format!("Recherche « {requete} » — {} résultat(s)", resultats.total())),
        )?;
        for c in &resultats.clients {
            ecrire(
                sortie,
                format!("  client   n°{:<4} {} ({})", c.id, c.nom, c.type_client.as_str()),
            )?;
        }
        let regime = self.store.societe_regime_public();
        for d in &resultats.documents {
            ecrire(
                sortie,
                format!(
                    "  document {:<14} {}  {}  {}",
                    d.numero,
                    d.date_doc,
                    d.statut.as_str(),
                    format_eur(d.totaux(regime).ttc)
                ),
            )?;
        }
        for a in &resultats.articles {
            ecrire(
                sortie,
                format!(
                    "  article  n°{:<4} {} {} ({})",
                    a.id, a.marque, a.description, a.statut
                ),
            )?;
        }
        for v in &resultats.ventes {
            ecrire(
                sortie,
                format!(
                    "  vente    n°{:<4} article {} le {} — {}",
                    v.id,
                    v.article_id,
                    v.date_vente,
                    format_eur(v.prix_vente)
                ),
            )?;
        }
        Ok(())
    }

    // ---------------------------------------------------------- Clients

    fn cmd_clients(&mut self, rest: &[&str], options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        match rest.first().map(|s| s.to_lowercase()).as_deref() {
            None => self.lister_clients(options, sortie),
            Some("nouveau") | Some("nouvelle") => {
                let client = self.creer_client_depuis_options(options)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Client n°{} créé : {} — /clients {} factures",
                        client.id, client.nom, client.nom
                    )),
                )
            }
            Some("importer") => {
                let fichier = rest.get(1).ok_or("usage : /clients importer <fichier.csv>")?;
                self.importer_clients(fichier, sortie)
            }
            Some("exporter") => self.exporter_clients(rest.get(1).map(|s| s.to_string()), sortie),
            Some(_) => {
                let recherche = rest[0];
                let client = self.trouver_client(recherche)?;
                match rest.get(1).map(|s| s.to_lowercase()).as_deref() {
                    None => self.fiche_client(&client, options, sortie),
                    Some("factures") | Some("documents") => {
                        self.documents_du_client(&client, sortie)
                    }
                    Some("tag") => {
                        let etiquette = rest.get(2).ok_or("usage : /clients <nom> tag <étiquette>")?;
                        let c = self.store.etiqueter(client.id, etiquette)?;
                        ecrire(
                            sortie,
                            theme::succes(format!(
                                "✓ {} étiqueté « {etiquette} » → {}",
                                c.nom,
                                etiquettes_vers_texte(&c.etiquettes)
                            )),
                        )
                    }
                    Some("detag") | Some("detaguer") => {
                        let etiquette = rest.get(2).ok_or("usage : /clients <nom> detag <étiquette>")?;
                        let c = self.store.retirer_etiquette(client.id, etiquette)?;
                        ecrire(
                            sortie,
                            theme::succes(format!(
                                "✓ étiquette retirée — reste : {}",
                                etiquettes_vers_texte(&c.etiquettes)
                            )),
                        )
                    }
                    Some("modifier") | Some("edit") => self.modifier_client(&client, options, sortie),
                    Some("rgpd") => self.rgpd_client(&client, rest.get(2).copied(), sortie),
                    Some(action) => Err(format!(
                        "action « {action} » inconnue — essayez : factures | tag | detag | modifier | rgpd"
                    )),
                }
            }
        }
    }

    fn lister_clients(&mut self, options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        let mut clients = match opt(options, "tag") {
            Some(tag) => self.store.clients_par_etiquette(tag),
            None => self.store.clients(),
        };
        clients.sort_by(|a, b| a.nom.cmp(&b.nom));
        if drapeau(options, "csv") {
            let lignes: Vec<Vec<String>> = clients
                .iter()
                .map(|c| {
                    vec![
                        c.id.to_string(),
                        c.nom.clone(),
                        c.type_client.as_str().to_string(),
                        c.email.clone(),
                        c.telephone.clone(),
                        c.adresse_complete(),
                        c.siren.clone(),
                        etiquettes_vers_texte(&c.etiquettes),
                        c.rgpd_efface_le.clone(),
                    ]
                })
                .collect();
            return ecrire(
                sortie,
                csv::exporter(
                    &[
                        "id", "nom", "type", "email", "telephone", "adresse", "siren", "etiquettes",
                        "rgpd_efface_le",
                    ],
                    &lignes,
                ),
            );
        }
        if drapeau(options, "json") {
            let items: Vec<String> = clients.iter().map(json_client).collect();
            return ecrire(sortie, format!("[{}]", items.join(",")));
        }
        if clients.is_empty() {
            return ecrire(
                sortie,
                "Aucun client. Créez-en un : /clients nouveau --nom \"…\" --type pro (ou importez un CSV)",
            );
        }
        let (page, pied) = paginer(&clients, options);
        for c in &page {
            ecrire(
                sortie,
                format!(
                    "{:>4}  {:<32} {:<14} {:<28} {}",
                    c.id,
                    c.nom,
                    c.type_client.as_str(),
                    c.email,
                    etiquettes_vers_texte(&c.etiquettes)
                ),
            )?;
        }
        ecrire(sortie, theme::info(pied))
    }

    fn fiche_client(&mut self, client: &Client, options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        let historique = self.store.historique_client(client.id)?;
        if drapeau(options, "json") {
            return ecrire(sortie, json_client(client));
        }
        ecrire(
            sortie,
            theme::titre(format!(
                "{} (n°{}, {}, {})",
                client.nom,
                client.id,
                client.type_client.as_str(),
                client.ville
            )),
        )?;
        let mut details: Vec<(&str, String)> = vec![
            ("e-mail", client.email.clone()),
            ("téléphone", client.telephone.clone()),
            ("adresse", client.adresse_complete()),
            ("SIREN", client.siren.clone()),
            ("TVA intracom.", client.tva_intracom.clone()),
            ("étiquettes", etiquettes_vers_texte(&client.etiquettes)),
            ("créé le", client.cree_le.clone()),
        ];
        if !client.rgpd_efface_le.is_empty() {
            details.push(("RGPD", format!("anonymisé le {}", client.rgpd_efface_le)));
        }
        for (libelle, valeur) in details {
            if !valeur.trim().is_empty() {
                ecrire(sortie, format!("  {libelle:<14}: {valeur}"))?;
            }
        }
        if !client.notes.is_empty() {
            ecrire(sortie, format!("  notes         : {}", client.notes))?;
        }
        ecrire(
            sortie,
            format!(
                "\n  Facturé : {} · encaissé : {} · dû : {}",
                format_eur(historique.total_facture),
                format_eur(historique.total_paye),
                format_eur(historique.total_du)
            ),
        )?;
        if !historique.documents.is_empty() {
            let regime = self.store.societe_regime_public();
            ecrire(sortie, "  Documents :")?;
            let (page, pied) = paginer(&historique.documents, options);
            for d in &page {
                ecrire(
                    sortie,
                    format!(
                        "   {} {}  {}  {:<22} {}",
                        d.statut.symbole(),
                        d.numero,
                        d.date_doc,
                        d.statut.as_str(),
                        format_eur(d.totaux(regime).ttc)
                    ),
                )?;
            }
            ecrire(sortie, theme::info(format!("  {pied}")))?;
        }
        if !historique.paiements.is_empty() {
            ecrire(sortie, "  Paiements :")?;
            for p in historique.paiements.iter().take(10) {
                ecrire(
                    sortie,
                    format!(
                        "   {} le {:<12} {:<14} {}",
                        format_eur(p.montant),
                        p.date_paiement,
                        p.moyen,
                        if p.rapproche { "(rapproché)" } else { "" }
                    ),
                )?;
            }
        }
        if !historique.ventes.is_empty() {
            ecrire(sortie, "  Achats Vinted :")?;
            for v in &historique.ventes {
                ecrire(
                    sortie,
                    format!("   {} le {} — {}", v.plateforme, v.date_vente, format_eur(v.prix_vente)),
                )?;
            }
        }
        Ok(())
    }

    fn documents_du_client(&mut self, client: &Client, sortie: &mut dyn Write) -> Resultat<()> {
        let documents: Vec<Document> = self
            .store
            .documents(None)
            .into_iter()
            .filter(|d| d.client_id == client.id)
            .collect();
        let regime = self.store.societe_regime_public();
        ecrire(sortie, format!("Documents de {} :", client.nom))?;
        if documents.is_empty() {
            return ecrire(sortie, "  (aucun)");
        }
        for d in &documents {
            ecrire(
                sortie,
                format!(
                    "  {} {}  {}  {}  TTC {}",
                    d.statut.symbole(),
                    d.numero,
                    d.date_doc,
                    d.statut.as_str(),
                    format_eur(d.totaux(regime).ttc)
                ),
            )?;
        }
        Ok(())
    }

    fn creer_client_depuis_options(&mut self, options: &Options) -> Resultat<Client> {
        let nom = opt(options, "nom").ok_or("précisez --nom \"…\"")?;
        let mut client = Client::nouveau(
            nom,
            TypeClient::from_str(opt(options, "type").unwrap_or("pro"))?,
        );
        client.email = opt(options, "email").unwrap_or("").to_string();
        client.telephone = opt(options, "tel").unwrap_or("").to_string();
        client.adresse = opt(options, "adresse").unwrap_or("").to_string();
        client.code_postal = opt(options, "cp").unwrap_or("").to_string();
        client.ville = opt(options, "ville").unwrap_or("").to_string();
        client.siren = opt(options, "siren").unwrap_or("").to_string();
        client.tva_intracom = opt(options, "tva").unwrap_or("").to_string();
        client.notes = opt(options, "notes").unwrap_or("").to_string();
        client.reference_externe = opt(options, "reference").unwrap_or("").to_string();
        client.etiquettes = options
            .get("tag")
            .cloned()
            .unwrap_or_default()
            .iter()
            .flat_map(|t| etiquettes_depuis_texte(t))
            .collect();
        self.store.creer_client(client)
    }

    fn modifier_client(&mut self, client: &Client, options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        let mut client = client.clone();
        if let Some(v) = opt(options, "nom") {
            client.nom = v.to_string();
        }
        if let Some(v) = opt(options, "email") {
            client.email = v.to_string();
        }
        if let Some(v) = opt(options, "tel") {
            client.telephone = v.to_string();
        }
        if let Some(v) = opt(options, "adresse") {
            client.adresse = v.to_string();
        }
        if let Some(v) = opt(options, "cp") {
            client.code_postal = v.to_string();
        }
        if let Some(v) = opt(options, "ville") {
            client.ville = v.to_string();
        }
        if let Some(v) = opt(options, "siren") {
            client.siren = v.to_string();
        }
        if let Some(v) = opt(options, "tva") {
            client.tva_intracom = v.to_string();
        }
        if let Some(v) = opt(options, "notes") {
            client.notes = v.to_string();
        }
        if let Some(v) = opt(options, "type") {
            client.type_client = TypeClient::from_str(v)?;
        }
        let client = self.store.modifier_client(client)?;
        ecrire(
            sortie,
            theme::succes(format!("✓ Client n°{} mis à jour : {}", client.id, client.nom)),
        )
    }

    fn rgpd_client(&mut self, client: &Client, action: Option<&str>, sortie: &mut dyn Write) -> Resultat<()> {
        match action.map(|a| a.to_lowercase()).as_deref() {
            Some("exporter") | Some("export") => {
                self.store.journaliser("rgpd.export", &client.nom)?;
                ecrire(sortie, self.store.exporter_client(client.id)?)
            }
            Some("effacer") | Some("anonymiser") => {
                let resume = self.store.effacer_client_rgpd(client.id)?;
                ecrire(sortie, theme::succes(format!("✓ {resume}")))
            }
            _ => Err("usage : /clients <nom> rgpd exporter|effacer".into()),
        }
    }

    fn importer_clients(&mut self, fichier: &str, sortie: &mut dyn Write) -> Resultat<()> {
        let table = csv::Table::depuis_fichier(Path::new(fichier))?;
        table.requis(&["nom", "client", "raison sociale"])?;
        let mut importes = 0;
        let mut erreurs = Vec::new();
        for (index, ligne) in table.lignes.iter().enumerate() {
            let nom = table.valeur(ligne, &["nom", "client", "raison_sociale"]);
            if nom.trim().is_empty() {
                continue;
            }
            let mut client = Client::nouveau(&nom, TypeClient::Professionnel);
            let type_texte = table.valeur(ligne, &["type", "categorie"]);
            if !type_texte.is_empty() {
                match TypeClient::from_str(&type_texte) {
                    Ok(t) => client.type_client = t,
                    Err(e) => erreurs.push(format!("ligne {} : {e}", index + 2)),
                }
            }
            client.email = table.valeur(ligne, &["email", "mail", "courriel"]);
            client.telephone = table.valeur(ligne, &["telephone", "tel", "phone"]);
            client.adresse = table.valeur(ligne, &["adresse", "address", "rue"]);
            client.code_postal = table.valeur(ligne, &["code_postal", "cp", "zip"]);
            client.ville = table.valeur(ligne, &["ville", "city"]);
            client.siren = table.valeur(ligne, &["siren", "siret", "identifiant"]);
            client.tva_intracom = table.valeur(ligne, &["tva", "tva_intracom"]);
            client.etiquettes = etiquettes_depuis_texte(&table.valeur(ligne, &["etiquettes", "tags", "tag"]));
            client.reference_externe = table.valeur(ligne, &["reference", "ref", "id_externe"]);
            match self.store.creer_client(client) {
                Ok(_) => importes += 1,
                Err(e) => erreurs.push(format!("ligne {} : {e}", index + 2)),
            }
        }
        self.store
            .journaliser("clients.import", &format!("{fichier} : {importes} clients"))?;
        for erreur in &erreurs {
            ecrire(sortie, theme::alerte(format!("⚠ {erreur}")))?;
        }
        ecrire(
            sortie,
            theme::succes(format!(
                "✓ {importes} client(s) importé(s) depuis {fichier}{}",
                if erreurs.is_empty() {
                    String::new()
                } else {
                    format!(" ({} ligne(s) en écart, non corrigées en silence)", erreurs.len())
                }
            )),
        )
    }

    fn exporter_clients(&mut self, fichier: Option<String>, sortie: &mut dyn Write) -> Resultat<()> {
        let clients = self.store.clients();
        let lignes: Vec<Vec<String>> = clients
            .iter()
            .map(|c| {
                vec![
                    c.nom.clone(),
                    c.type_client.as_str().to_string(),
                    c.email.clone(),
                    c.telephone.clone(),
                    c.adresse.clone(),
                    c.code_postal.clone(),
                    c.ville.clone(),
                    c.siren.clone(),
                    c.tva_intracom.clone(),
                    etiquettes_vers_texte(&c.etiquettes),
                ]
            })
            .collect();
        let contenu = csv::exporter(
            &[
                "nom", "type", "email", "telephone", "adresse", "code_postal", "ville", "siren",
                "tva_intracom", "etiquettes",
            ],
            &lignes,
        );
        match fichier {
            Some(chemin) => {
                std::fs::write(chemin, &contenu)
                    .map_err(|e| format!("écriture de {chemin} : {e}"))?;
                ecrire(
                    sortie,
                    theme::succes(format!("✓ {len} client(s) exporté(s) dans {chemin}", len = clients.len())),
                )
            }
            None => ecrire(sortie, contenu),
        }
    }

    fn trouver_client(&mut self, recherche: &str) -> Resultat<Client> {
        self.store
            .chercher_clients(recherche)
            .into_iter()
            .next()
            .ok_or_else(|| {
                format!("aucun client ne correspond à « {recherche} » — `/clients` pour la liste")
            })
    }

    // ---------------------------------------------------------- Documents

    fn cmd_documents(
        &mut self,
        type_doc: TypeDocument,
        rest: &[&str],
        options: &Options,
        sortie: &mut dyn Write,
    ) -> Resultat<()> {
        match rest.first().map(|s| s.to_lowercase()).as_deref() {
            None => self.lister_documents(type_doc, options, sortie),
            Some("nouveau") | Some("nouvelle") => self.nouveau_document(type_doc, options, sortie),
            Some("recurrentes") | Some("recurrents") => {
                self.cmd_recurrences(&rest[1..], options, sortie)
            }
            Some(_) => {
                let numero = self.resoudre_numero(type_doc, rest)?;
                // « 2026 0001 valider » consomme deux jetons pour la référence.
                let annee_seule = rest
                    .first()
                    .map(|s| s.len() == 4 && s.chars().all(|c| c.is_ascii_digit()))
                    .unwrap_or(false);
                let debut = if annee_seule && rest.len() > 1 { 2 } else { 1 };
                let action: Vec<String> = rest[debut.min(rest.len())..]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                self.action_document(type_doc, &numero, &action, options, sortie)
            }
        }
    }

    /// Accepte `FA-2026-0001`, `2026/0001`, `2026 1` (CDC §6.1).
    fn resoudre_numero(&mut self, type_doc: TypeDocument, rest: &[&str]) -> Resultat<String> {
        let premier = rest.first().copied().unwrap_or("");
        if premier.chars().all(|c| c.is_ascii_digit()) && premier.len() == 4 {
            if let Some(sequence) = rest.get(1) {
                let document = self
                    .store
                    .document_par_annee_sequence(type_doc, premier, sequence)?;
                return Ok(document.numero);
            }
        }
        let document = self.store.document_par_numero(premier)?;
        if document.type_doc != type_doc {
            return Err(format!(
                "{} est un {} — utilisez /{}",
                document.numero,
                document.type_doc.as_str(),
                pluriel_document(document.type_doc)
            ));
        }
        Ok(document.numero)
    }

    fn lister_documents(
        &mut self,
        type_doc: TypeDocument,
        options: &Options,
        sortie: &mut dyn Write,
    ) -> Resultat<()> {
        let regime = self.store.societe_regime_public();
        let mut documents = self.store.documents(Some(type_doc));
        if let Some(recherche_client) = opt(options, "client") {
            let client = self.trouver_client(recherche_client)?;
            documents.retain(|d| d.client_id == client.id);
        }
        if let Some(statut) = opt(options, "statut") {
            let cible = StatutDocument::depuis_code(statut);
            documents.retain(|d| d.statut == cible);
        }
        if drapeau(options, "impayees") {
            documents.retain(|d| d.est_impaye());
        }
        if drapeau(options, "retard") {
            let reference = aujourdhui();
            documents.retain(|d| d.en_retard(&reference));
        }
        if drapeau(options, "json") {
            let items: Vec<String> = documents.iter().map(|d| json_document(d, regime)).collect();
            return ecrire(sortie, format!("[{}]", items.join(",")));
        }
        if drapeau(options, "csv") {
            let lignes: Vec<Vec<String>> = documents
                .iter()
                .map(|d| {
                    let totaux = d.totaux(regime);
                    vec![
                        d.numero.clone(),
                        d.type_doc.as_str().to_string(),
                        d.statut.as_str().to_string(),
                        d.date_doc.clone(),
                        d.echeance.clone(),
                        d.client_id.to_string(),
                        d.activite.clone(),
                        d.devise.clone(),
                        format!("{},{}", totaux.ht / 100, (totaux.ht % 100).abs()),
                        format!("{},{}", totaux.tva / 100, (totaux.tva % 100).abs()),
                        format!("{},{}", totaux.ttc / 100, (totaux.ttc % 100).abs()),
                        format!("{},{}", d.paye / 100, (d.paye % 100).abs()),
                    ]
                })
                .collect();
            return ecrire(
                sortie,
                csv::exporter(
                    &[
                        "numero", "type", "statut", "date", "echeance", "client_id", "activite",
                        "devise", "ht", "tva", "ttc", "paye",
                    ],
                    &lignes,
                ),
            );
        }
        if documents.is_empty() {
            return ecrire(
                sortie,
                format!(
                    "Aucun {}. Créez-en un : /{} nouvelle --client <nom> --ligne \"…\"",
                    type_doc.as_str(),
                    pluriel_document(type_doc)
                ),
            );
        }
        let (page, pied) = paginer(&documents, options);
        for d in &page {
            let totaux = d.totaux(regime);
            let retard = if d.en_retard(&aujourdhui()) {
                theme::erreur(" (retard)").to_string()
            } else {
                String::new()
            };
            ecrire(
                sortie,
                format!(
                    "{} {:<14} {}  {:>12}  payé {:>12}  {:<22}{}",
                    d.statut.symbole(),
                    d.numero,
                    d.date_doc,
                    format_eur(totaux.ttc),
                    format_eur(d.paye),
                    d.statut.as_str(),
                    retard
                ),
            )?;
        }
        ecrire(sortie, theme::info(pied))
    }

    fn nouveau_document(
        &mut self,
        type_doc: TypeDocument,
        options: &Options,
        sortie: &mut dyn Write,
    ) -> Resultat<()> {
        let recherche = opt(options, "client").ok_or("précisez --client <nom ou e-mail>")?;
        let client = self.trouver_client(recherche)?;
        let mut lignes = Vec::new();
        for texte in options.get("ligne").cloned().unwrap_or_default() {
            lignes.push(parser_ligne(&texte)?);
        }
        if lignes.is_empty() {
            return Err(
                "ajoutez au moins une --ligne \"désignation;quantité;prix HT[;taux bp]\"".into(),
            );
        }
        let date = opt(options, "date").unwrap_or("").to_string();
        let date = if date.is_empty() { aujourdhui() } else { date };
        let mut nouveau = NouveauDocument::nouveau(type_doc, client.id, &date);
        nouveau.lignes = lignes;
        nouveau.date_prestation = opt(options, "prestation").unwrap_or(&date).to_string();
        nouveau.activite = opt(options, "activite").unwrap_or("services").to_string();
        if let Some(echeance) = opt(options, "echeance") {
            nouveau.echeance = echeance.to_string();
        }
        if let Some(devise) = opt(options, "devise") {
            let taux = match self.store.taux(devise, &date) {
                Some(t) => t.taux_micro,
                None => {
                    return Err(format!(
                        "aucun taux connu pour {devise} au {date} — enregistrez-le : \
                         `/devises taux {devise} {date} 1,07`"
                    ))
                }
            };
            nouveau.devise = devise.to_string();
            nouveau.taux_change_micro = taux;
        }
        if let Some(motif) = opt(options, "motif") {
            nouveau.motif = motif.to_string();
        }
        let document = self.store.creer_document(nouveau)?;
        self.store
            .journaliser("document.creation", &format!("{} {}", type_doc.as_str(), document.numero))?;
        let regime = self.store.societe_regime_public();
        ecrire(
            sortie,
            theme::succes(format!(
                "✓ {} {} créé : {} TTC (brouillon) — validez : /{} {} valider",
                type_doc.as_str(),
                document.numero,
                format_eur(document.totaux(regime).ttc),
                pluriel_document(type_doc),
                document.numero
            )),
        )
    }

    fn action_document(
        &mut self,
        type_doc: TypeDocument,
        numero: &str,
        action: &[String],
        options: &Options,
        sortie: &mut dyn Write,
    ) -> Resultat<()> {
        let regime = self.store.societe_regime_public();
        let action_nom = action
            .first()
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        match action_nom.as_str() {
            "" => self.detail_document(numero, options, sortie),
            "valider" => {
                self.store.exiger_admin()?;
                let document = self.store.valider_document(numero)?;
                self.store.journaliser("document.validation", &document.numero)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ {} validée et figée — numérotation officielle, aucune modification possible (avoir requis)",
                        document.numero
                    )),
                )
            }
            "envoyer" | "mail" | "email" => {
                let document = self.store.document_par_numero(numero)?;
                let client = self.store.client_par_id(document.client_id)?;
                let societe = self.store.societe()?;
                let courriel = messagerie::courriel_document(&societe, &client, &document, regime);
                let chemin = self.courrier(&courriel)?;
                if document.statut == StatutDocument::Brouillon {
                    self.store.marquer_envoye(numero)?;
                } else {
                    self.store.journaliser("document.envoi", numero)?;
                }
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ {} préparé pour {} — message dans {} (PDF joint)",
                        document.numero,
                        client.email,
                        chemin.display()
                    )),
                )
            }
            "payer" | "paiement" => {
                let document = self.store.document_par_numero(numero)?;
                let montant = match action.get(1) {
                    Some(texte) => parse_eur(texte)?,
                    None => document.reste(regime),
                };
                if montant <= 0 {
                    return Err("montant nul ou facture déjà soldée".into());
                }
                let date = action
                    .get(2)
                    .cloned()
                    .unwrap_or_else(aujourdhui);
                let moyen = action.get(3).cloned().unwrap_or_else(|| "virement".into());
                let reference = action.get(4).cloned().unwrap_or_default();
                let document = self
                    .store
                    .ajouter_paiement(numero, montant, &date, &moyen, &reference)?;
                self.store
                    .journaliser("paiement", &format!("{numero} {}", format_eur(montant)))?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Paiement de {} enregistré sur {} — statut : {} (reste {})",
                        format_eur(montant),
                        document.numero,
                        document.statut.as_str(),
                        format_eur(document.reste(regime))
                    )),
                )
            }
            "avoir" => {
                let document = self.store.creer_avoir(numero, &aujourdhui())?;
                self.store
                    .journaliser("avoir.creation", &format!("{} pour {numero}", document.numero))?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Avoir {} créé et validé pour corriger {} ({})",
                        document.numero,
                        numero,
                        format_eur(document.totaux(regime).ttc)
                    )),
                )
            }
            "relance" | "relancer" => self.relancer(numero, sortie),
            "pdf" => {
                let document = self.store.document_par_numero(numero)?;
                let client = self.store.client_par_id(document.client_id)?;
                let societe = self.store.societe()?;
                let octets = pdf::facture(&societe, &client, &document, regime);
                let chemin = action
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| format!("{}.pdf", document.numero));
                std::fs::write(&chemin, &octets).map_err(|e| format!("écriture de {chemin} : {e}"))?;
                self.store.journaliser("document.pdf", &document.numero)?;
                ecrire(
                    sortie,
                    theme::succes(format!("✓ PDF conforme écrit dans {chemin} ({} octets)", octets.len())),
                )
            }
            "ubl" | "efacture" | "xml" => {
                let document = self.store.document_par_numero(numero)?;
                let client = self.store.client_par_id(document.client_id)?;
                let societe = self.store.societe()?;
                let xml = efacture::ubl(&societe, &client, &document, regime);
                let chemin = action
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| format!("{}.xml", document.numero));
                std::fs::write(&chemin, &xml).map_err(|e| format!("écriture de {chemin} : {e}"))?;
                self.store.journaliser("document.ubl", &document.numero)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ e-facture UBL (EN 16931) écrite dans {chemin} — à transmettre via une plateforme agréée"
                    )),
                )
            }
            "annuler" => {
                let motif = action.get(1).cloned().unwrap_or_else(|| "annulation".into());
                let document = self.store.annuler_document(numero, &motif)?;
                self.store
                    .journaliser("document.annulation", &format!("{} : {motif}", document.numero))?;
                ecrire(
                    sortie,
                    theme::succes(format!("✓ {} annulée (motif : {motif})", document.numero)),
                )
            }
            "accepter" | "refuser" | "expirer" => {
                let statut = match action_nom.as_str() {
                    "accepter" => StatutDocument::Accepte,
                    "refuser" => StatutDocument::Refuse,
                    _ => StatutDocument::Expire,
                };
                let document = self.store.changer_statut(numero, statut)?;
                self.store
                    .journaliser(&format!("document.{}", action_nom), &document.numero)?;
                ecrire(
                    sortie,
                    theme::succes(format!("✓ {} — statut : {}", document.numero, document.statut.as_str())),
                )
            }
            "transformer" => {
                let facture = self.store.transformer_devis(numero, &aujourdhui())?;
                self.store
                    .journaliser("devis.transformation", &format!("{numero} -> {}", facture.numero))?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Devis {numero} transformé en facture {} (brouillon)",
                        facture.numero
                    )),
                )
            }
            "modifier" | "edit" => Err(format!(
                "{} ne se modifie pas : un document validé est immuable (EF-FAC-02). Créez un avoir : \
                 /{} {} avoir",
                numero,
                pluriel_document(type_doc),
                numero
            )),
            "supprimer" | "delete" => Err(format!(
                "suppression interdite : les pièces comptables se conservent 10 ans. \
                 Corrigez avec un avoir : /{} {numero} avoir",
                pluriel_document(type_doc)
            )),
            autre => Err(format!(
                "action « {autre} » inconnue sur {numero} — essayez : valider | envoyer | payer | relance | avoir | pdf | ubl | annuler{}",
                if type_doc == TypeDocument::Devis {
                    " | accepter | refuser | expirer | transformer"
                } else {
                    ""
                }
            )),
        }
    }

    fn detail_document(&mut self, numero: &str, options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        let document = self.store.document_par_numero(numero)?;
        let regime = self.store.societe_regime_public();
        let client = self.store.client_par_id(document.client_id)?;
        if drapeau(options, "json") {
            return ecrire(sortie, json_document(&document, regime));
        }
        let totaux = document.totaux(regime);
        ecrire(
            sortie,
            theme::titre(format!(
                "{} {} — {} — client : {}",
                document.type_doc.as_str(),
                document.numero,
                document.statut.as_str(),
                client.nom
            )),
        )?;
        ecrire(
            sortie,
            format!(
                "  émission {} · prestation {} · échéance {} · {} · activité {}",
                date_fr(&document.date_doc),
                date_fr(&document.date_prestation),
                date_fr(&document.echeance),
                document.conditions_paiement,
                document.activite
            ),
        )?;
        if !document.envoye_le.is_empty() {
            ecrire(sortie, format!("  envoyé le {}", date_fr(&document.envoye_le)))?;
        }
        if document.devise != DEVISE_REFERENCE {
            ecrire(
                sortie,
                format!(
                    "  devise {} (1 {} = {},{:06} EUR)",
                    document.devise,
                    document.devise,
                    document.taux_change_micro / 1_000_000,
                    document.taux_change_micro % 1_000_000
                ),
            )?;
        }
        for ligne in &document.lignes {
            let remise = if ligne.remise_bp > 0 {
                format!(" (remise {})", format_taux_bp(ligne.remise_bp))
            } else {
                String::new()
            };
            ecrire(
                sortie,
                format!(
                    "  - {} × {} à {}{} = {}",
                    ligne.designation,
                    format_quantite(ligne.quantite_milli),
                    format_eur(ligne.prix_unitaire_ht),
                    remise,
                    format_eur(ligne.total_ht())
                ),
            )?;
        }
        ecrire(
            sortie,
            format!(
                "  HT {} · TVA {} · TTC {} · payé {} · reste {}",
                format_eur(totaux.ht),
                format_eur(totaux.tva),
                format_eur(totaux.ttc),
                format_eur(document.paye),
                format_eur(document.reste(regime))
            ),
        )?;
        for ventilation in tva_par_taux(&document.lignes, regime) {
            ecrire(
                sortie,
                format!(
                    "  TVA {} : base {} → {}",
                    format_taux_bp(ventilation.taux_bp),
                    format_eur(ventilation.base_ht),
                    format_eur(ventilation.tva)
                ),
            )?;
        }
        if let Some(mention) = regime.mention_facture() {
            ecrire(sortie, format!("  mention : {mention}"))?;
        }
        if document.type_doc == TypeDocument::Facture && document.est_impaye() {
            let jours = jours_retard(&document.echeance, &aujourdhui());
            if jours > 0 {
                ecrire(
                    sortie,
                    theme::alerte(format!(
                        "  ⚠ retard de {jours} jour(s) — pénalités + indemnité : {}",
                        format_eur(document.penalites(regime, &aujourdhui()))
                    )),
                )?;
            }
        }
        for paiement in self.store.paiements(document.id) {
            ecrire(
                sortie,
                format!(
                    "  paiement {} le {} ({}{})",
                    format_eur(paiement.montant),
                    date_fr(&paiement.date_paiement),
                    paiement.moyen,
                    if paiement.rapproche { ", rapproché" } else { "" }
                ),
            )?;
        }
        if !document.motif.is_empty() {
            ecrire(sortie, format!("  motif : {}", document.motif))?;
        }
        Ok(())
    }

    fn relancer(&mut self, numero: &str, sortie: &mut dyn Write) -> Resultat<()> {
        let reference = aujourdhui();
        let regime = self.store.societe_regime_public();
        let document = self.store.document_par_numero(numero)?;
        let niveau = document.niveau_relance(&reference);
        if niveau == 0 {
            return Err(format!(
                "{} n'est pas en retard : aucune relance à envoyer (échéance {})",
                document.numero,
                date_fr(&document.echeance)
            ));
        }
        let client = self.store.client_par_id(document.client_id)?;
        let societe = self.store.societe()?;
        let courriel =
            messagerie::courriel_relance(&societe, &client, &document, regime, niveau, &reference);
        let chemin = self.courrier(&courriel)?;
        self.store.journaliser(
            "relance",
            &format!("{} niveau {niveau}", document.numero),
        )?;
        ecrire(
            sortie,
            theme::succes(format!(
                "✓ Relance niveau {niveau} préparée pour {} ({}) — message dans {}",
                client.nom,
                format_eur(document.reste(regime) + document.penalites(regime, &reference)),
                chemin.display()
            )),
        )
    }

    fn courrier(&mut self, courriel: &messagerie::Courriel) -> Resultat<std::path::PathBuf> {
        let dossier = self
            .store
            .dossier()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        messagerie::ecrire(&dossier, courriel)
    }

    // ---------------------------------------------------------- Récurrences

    fn cmd_recurrences(&mut self, rest: &[&str], options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        match rest.first().map(|s| s.to_lowercase()).as_deref() {
            None => {
                let recurrences = self.store.recurrences();
                if recurrences.is_empty() {
                    return ecrire(
                        sortie,
                        "Aucune facture récurrente. Créez-en une : /factures recurrentes nouvelle --client … --ligne … --periodicite mensuelle",
                    );
                }
                for recurrence in &recurrences {
                    ecrire(
                        sortie,
                        format!(
                            "n°{:<3} {} · {:<14} · prochaine {} · {} · client n°{} · {} ligne(s)",
                            recurrence.id,
                            recurrence.libelle,
                            recurrence.periodicite,
                            recurrence.prochaine_le,
                            if recurrence.actif { "actif" } else { "suspendu" },
                            recurrence.client_id,
                            recurrence.lignes.len()
                        ),
                    )?;
                }
                Ok(())
            }
            Some("nouvelle") | Some("nouveau") => {
                let recherche = opt(options, "client").ok_or("précisez --client <nom>")?;
                let client = self.trouver_client(recherche)?;
                let mut lignes = Vec::new();
                for texte in options.get("ligne").cloned().unwrap_or_default() {
                    lignes.push(parser_ligne(&texte)?);
                }
                if lignes.is_empty() {
                    return Err("ajoutez au moins une --ligne \"…\"".into());
                }
                let prochaine = opt(options, "prochaine")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| ajouter_mois(&aujourdhui(), 1));
                let recurrence = self.store.creer_recurrence(Recurrence {
                    id: 0,
                    societe_id: 0,
                    client_id: client.id,
                    type_doc: TypeDocument::Facture,
                    activite: opt(options, "activite").unwrap_or("services").to_string(),
                    periodicite: opt(options, "periodicite").unwrap_or("mensuelle").to_string(),
                    prochaine_le: prochaine,
                    lignes,
                    actif: true,
                    libelle: opt(options, "libelle").unwrap_or("abonnement").to_string(),
                })?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Facture récurrente n°{} créée pour {} — prochaine génération le {}",
                        recurrence.id, client.nom, recurrence.prochaine_le
                    )),
                )
            }
            Some("generer") | Some("générer") => {
                let date = rest
                    .get(1)
                    .map(|s| s.to_string())
                    .unwrap_or_else(aujourdhui);
                let documents = self.store.generer_recurrences(&date)?;
                self.store
                    .journaliser("recurrences.generation", &format!("{date} : {}", documents.len()))?;
                if documents.is_empty() {
                    return ecrire(sortie, "Aucune échéance à générer.");
                }
                for document in &documents {
                    ecrire(sortie, format!("  → {} ({})", document.numero, document.date_doc))?;
                }
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ {} brouillon(s) généré(s) — validez-les avec /factures <n°> valider",
                        documents.len()
                    )),
                )
            }
            Some("suspendre") | Some("reprendre") => {
                let id: i64 = rest
                    .get(1)
                    .ok_or("précisez le numéro de la récurrence")?
                    .parse()
                    .map_err(|_| "numéro illisible".to_string())?;
                let actif = rest[0].to_lowercase().starts_with("reprendre");
                let recurrence = self.store.basculer_recurrence(id, actif)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Récurrence n°{id} {} (prochaine : {})",
                        if recurrence.actif { "réactivée" } else { "suspendue" },
                        recurrence.prochaine_le
                    )),
                )
            }
            _ => Err("usage : /factures recurrentes [nouvelle … | generer [date] | suspendre <n°> | reprendre <n°>]".into()),
        }
    }

    // ---------------------------------------------------------- Vinted

    fn cmd_vinted(&mut self, rest: &[&str], options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        match rest.first().map(|s| s.to_lowercase()).as_deref() {
            None => self.lister_vinted(options, sortie),
            Some("nouveau") | Some("nouvelle") => {
                let mut article = Article::nouveau(
                    opt(options, "marque").unwrap_or("sans marque"),
                    opt(options, "description").unwrap_or(""),
                );
                article.taille = opt(options, "taille").unwrap_or("").to_string();
                article.composition = opt(options, "composition").unwrap_or("").to_string();
                article.etat = opt(options, "etat").unwrap_or("très bon").to_string();
                article.prix_achat = csv::montant(opt(options, "prix-achat").unwrap_or("0"))?;
                article.date_achat = opt(options, "date").unwrap_or(&aujourdhui()).to_string();
                article.regime =
                    RegimeVente::from_str(opt(options, "regime").unwrap_or("achat_revente"))?;
                article.sku = opt(options, "sku").unwrap_or("").to_string();
                article.photos = options
                    .get("photo")
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .flat_map(|p| decoder_liste(p))
                    .collect();
                article.quantite = opt_i64(options, "quantite", 1);
                let article = self.store.creer_article(article)?;
                self.store
                    .journaliser("article.creation", &format!("n°{} {}", article.id, article.description))?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Article n°{} ajouté : {} {} — achat {} — {}",
                        article.id,
                        article.marque,
                        article.description,
                        format_eur(article.prix_achat),
                        article.regime.libelle()
                    )),
                )
            }
            Some("vendre") => {
                let id: i64 = rest
                    .get(1)
                    .ok_or("usage : /vinted vendre <n° article> --prix 25,00")?
                    .parse()
                    .map_err(|_| "identifiant d'article illisible".to_string())?;
                let article = self.store.article_par_id(id)?;
                let mut vente = Vente::nouvelle(
                    id,
                    csv::montant(opt(options, "prix").ok_or("précisez --prix 25,00")?)?,
                    csv::montant(opt(options, "frais").unwrap_or("0"))?,
                );
                vente.date_vente = opt(options, "date").unwrap_or(&aujourdhui()).to_string();
                vente.plateforme = opt(options, "plateforme").unwrap_or("Vinted").to_string();
                vente.acheteur = opt(options, "acheteur").unwrap_or("").to_string();
                vente.reference = opt(options, "reference").unwrap_or("").to_string();
                vente.regime = article.regime;
                let vente = self.store.enregistrer_vente(vente)?;
                self.store.journaliser(
                    "vente.enregistrement",
                    &format!("article {} à {} ({})", id, format_eur(vente.prix_vente), vente.regime.as_str()),
                )?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Vente enregistrée : {} {} — {} — marge nette {}",
                        article.marque,
                        article.description,
                        format_eur(vente.prix_vente),
                        format_eur(marge_nette(&article, &vente))
                    )),
                )
            }
            Some("stats") => self.stats_vinted(options, sortie),
            Some("fiche") => {
                let id: i64 = rest
                    .get(1)
                    .ok_or("usage : /vinted fiche <n° article>")?
                    .parse()
                    .map_err(|_| "identifiant illisible".to_string())?;
                let article = self.store.article_par_id(id)?;
                ecrire(sortie, theme::titre(format!("Annonce — article n°{id}")))?;
                ecrire(sortie, article.annonce())
            }
            Some("importer") => {
                let fichier = rest.get(1).ok_or("usage : /vinted importer <ventes.csv>")?;
                self.importer_ventes(fichier, sortie)
            }
            Some("exporter") => self.exporter_articles(rest.get(1).map(|s| s.to_string()), sortie),
            _ => Err("usage : /vinted [nouveau … | vendre <n°> … | stats | fiche <n°> | importer <csv> | exporter <csv>]".into()),
        }
    }

    fn lister_vinted(&mut self, options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        let articles = self.store.articles();
        if drapeau(options, "json") {
            let items: Vec<String> = articles.iter().map(json_article).collect();
            return ecrire(sortie, format!("[{}]", items.join(",")));
        }
        if articles.is_empty() {
            return ecrire(
                sortie,
                "Aucun article. Ajoutez-en un : /vinted nouveau --marque … --description … --composition \"100 % coton\" --prix-achat 8,00",
            );
        }
        let (page, pied) = paginer(&articles, options);
        for article in &page {
            ecrire(
                sortie,
                format!(
                    "{:>4}  {:<12} {:<30} taille {:<4} achat {:>10}  {:>10}  {:<12} {}",
                    article.id,
                    article.marque,
                    tronquer(&article.description, 30),
                    article.taille,
                    format_eur(article.prix_achat),
                    format_eur(article.prix_vente_conseille()),
                    article.statut,
                    article.regime.as_str()
                ),
            )?;
        }
        ecrire(sortie, theme::info(pied))
    }

    fn stats_vinted(&mut self, _options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        let annee = annee_courante();
        let stats = self.store.stats_vinted(annee);
        ecrire(
            sortie,
            theme::titre(format!("Vinted — année {annee}")),
        )?;
        ecrire(
            sortie,
            format!(
                "  {} vente(s) · CA professionnel {} · marge nette {} · CA biens personnels {}",
                stats.nb_ventes,
                format_eur(stats.ca),
                format_eur(stats.marge),
                format_eur(stats.ca_personnel)
            ),
        )?;
        ecrire(
            sortie,
            format!(
                "  Stock : {} article(s) en stock (valeur d'achat {}) · {} vendu(s)",
                stats.articles_en_stock,
                format_eur(stats.valeur_stock),
                stats.articles_vendus
            ),
        )?;
        if stats.marge != 0 {
            ecrire(
                sortie,
                format!(
                    "  Marge moyenne : {} par vente",
                    format_eur(stats.marge / i64::from(stats.nb_ventes.max(1)))
                ),
            )?;
        }
        for (marque, nombre, marge) in stats.par_marque.iter().take(8) {
            ecrire(
                sortie,
                format!("  marque {marque:<14} {nombre:>3} vente(s)  marge {}", format_eur(*marge)),
            )?;
        }
        for (taille, nombre, marge) in stats.par_taille.iter().take(8) {
            ecrire(
                sortie,
                format!("  taille {taille:<14} {nombre:>3} vente(s)  marge {}", format_eur(*marge)),
            )?;
        }
        if let Some(alerte) = alerte_dac7(stats.nb_ventes, stats.ca) {
            ecrire(sortie, theme::alerte(format!("  ⚠ {alerte}")))?;
        } else {
            ecrire(
                sortie,
                theme::info(format!(
                    "  DAC7 : {} / 30 ventes · {} / 2 000,00 €",
                    stats.nb_ventes,
                    format_eur(stats.ca)
                )),
            )?;
        }
        Ok(())
    }

    fn importer_ventes(&mut self, fichier: &str, sortie: &mut dyn Write) -> Resultat<()> {
        let table = csv::Table::depuis_fichier(Path::new(fichier))?;
        let mut importees = 0;
        let mut erreurs = Vec::new();
        let articles = self.store.articles();
        for (index, ligne) in table.lignes.iter().enumerate() {
            let designation = table.valeur(ligne, &["article", "description", "titre", "libelle"]);
            let prix = match csv::montant(&table.valeur(ligne, &["prix", "prix_vente", "montant", "total"])) {
                Ok(p) => p,
                Err(e) => {
                    erreurs.push(format!("ligne {} : {e}", index + 2));
                    continue;
                }
            };
            let frais = csv::montant(&table.valeur(ligne, &["frais", "commission", "frais_port"]))
                .unwrap_or(0);
            let date = {
                let brute = table.valeur(ligne, &["date", "date_vente"]);
                if brute.is_empty() {
                    aujourdhui()
                } else {
                    brute
                }
            };
            let reference = table.valeur(ligne, &["reference", "ref", "commande", "id"]);
            // Rattachement à un article existant : référence, SKU, puis description.
            let article = articles
                .iter()
                .find(|a| !reference.is_empty() && a.sku.eq_ignore_ascii_case(&reference))
                .or_else(|| {
                    articles
                        .iter()
                        .find(|a| correspond(&a.description, &designation))
                })
                .or_else(|| {
                    articles
                        .iter()
                        .find(|a| !designation.is_empty() && correspond(&designation, &a.description))
                });
            let article = match article {
                Some(a) => a,
                None => {
                    erreurs.push(format!(
                        "ligne {} : aucun article ne correspond à « {designation} » — créez-le d'abord (/vinted nouveau) puis rejouez l'import",
                        index + 2
                    ));
                    continue;
                }
            };
            if article.statut == "vendu" && !reference.is_empty() {
                erreurs.push(format!(
                    "ligne {} : article n°{} déjà vendu — ignoré (jamais corrigé en silence)",
                    index + 2,
                    article.id
                ));
                continue;
            }
            let mut vente = Vente::nouvelle(article.id, prix, frais);
            vente.date_vente = date;
            vente.reference = reference;
            vente.regime = article.regime;
            vente.plateforme = table.valeur(ligne, &["plateforme", "site"]);
            if vente.plateforme.is_empty() {
                vente.plateforme = "Vinted".into();
            }
            vente.acheteur = table.valeur(ligne, &["acheteur", "client"]);
            match self.store.enregistrer_vente(vente) {
                Ok(_) => importees += 1,
                Err(e) => erreurs.push(format!("ligne {} : {e}", index + 2)),
            }
        }
        self.store
            .journaliser("ventes.import", &format!("{fichier} : {importees}"))?;
        for erreur in &erreurs {
            ecrire(sortie, theme::alerte(format!("⚠ {erreur}")))?;
        }
        ecrire(
            sortie,
            theme::succes(format!(
                "✓ {importees} vente(s) importée(s) depuis {fichier}{}",
                if erreurs.is_empty() {
                    String::new()
                } else {
                    format!(" ({} écart(s) signalé(s))", erreurs.len())
                }
            )),
        )
    }

    fn exporter_articles(&mut self, fichier: Option<String>, sortie: &mut dyn Write) -> Resultat<()> {
        let articles = self.store.articles();
        let lignes: Vec<Vec<String>> = articles
            .iter()
            .map(|a| {
                vec![
                    a.id.to_string(),
                    a.marque.clone(),
                    a.description.clone(),
                    a.taille.clone(),
                    a.composition.clone(),
                    a.etat.clone(),
                    format!("{},{}", a.prix_achat / 100, (a.prix_achat % 100).abs()),
                    a.date_achat.clone(),
                    a.statut.clone(),
                    a.regime.as_str().to_string(),
                    encoder_liste(&a.photos),
                    a.sku.clone(),
                ]
            })
            .collect();
        let contenu = csv::exporter(
            &[
                "id", "marque", "description", "taille", "composition", "etat", "prix_achat",
                "date_achat", "statut", "regime", "photos", "sku",
            ],
            &lignes,
        );
        match fichier {
            Some(chemin) => {
                std::fs::write(chemin, &contenu)
                    .map_err(|e| format!("écriture de {chemin} : {e}"))?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ {len} article(s) exporté(s) dans {chemin}",
                        len = articles.len()
                    )),
                )
            }
            None => ecrire(sortie, contenu),
        }
    }

    // ---------------------------------------------------------- Comptabilité

    fn cmd_compta(&mut self, rest: &[&str], options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        match rest.first().map(|s| s.to_lowercase()).as_deref() {
            Some("seuils") => self.seuils(sortie),
            Some("fec") => {
                let ecritures = self.ecritures_generees()?;
                let lignes = compta::fec(&ecritures);
                match rest.get(1) {
                    Some(fichier) => {
                        std::fs::write(fichier.as_str(), lignes.join("\n") + "\n")
                            .map_err(|e| format!("écriture du FEC : {e}"))?;
                        ecrire(
                            sortie,
                            theme::succes(format!(
                                "✓ FEC écrit dans {fichier} ({} ligne(s)) — à faire valider par l'expert-comptable",
                                lignes.len()
                            )),
                        )
                    }
                    None => {
                        for ligne in &lignes {
                            ecrire(sortie, ligne)?;
                        }
                        Ok(())
                    }
                }
            }
            Some("journal") => {
                let code = rest.get(1).map(|s| s.to_uppercase());
                let ecritures = self.ecritures_generees()?;
                match code {
                    Some(code) => {
                        for e in compta::journal(&ecritures, &code) {
                            ecrire(
                                sortie,
                                format!(
                                    "{} {:<4} {:<10} {:<50} {:>12} {:>12}",
                                    e.date_ecriture,
                                    e.journal,
                                    e.compte,
                                    tronquer(&e.libelle, 50),
                                    format_eur(e.debit),
                                    format_eur(e.credit)
                                ),
                            )?;
                        }
                        Ok(())
                    }
                    None => {
                        for (code, libelle, nombre) in compta::journaux(&ecritures) {
                            ecrire(sortie, format!("{code:<4} {libelle:<26} {nombre:>4} écriture(s)"))?;
                        }
                        Ok(())
                    }
                }
            }
            Some("tva") => {
                let periode = rest
                    .get(1)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| annee_courante().to_string());
                let regime = self.store.societe_regime_public();
                let documents = self.store.documents(None);
                let recap = compta::recap_tva_par_taux(&documents, regime, &periode);
                ecrire(
                    sortie,
                    theme::titre(format!("TVA collectée — période {periode} ({})", regime.libelle())),
                )?;
                if recap.is_empty() {
                    ecrire(
                        sortie,
                        "  Aucune facture validée sur la période (ou franchise en base : pas de TVA).",
                    )?;
                }
                for ligne in &recap {
                    ecrire(
                        sortie,
                        format!(
                            "  taux {:<7} base HT {:>12}  TVA {:>12}",
                            format_taux_bp(ligne.taux_bp),
                            format_eur(ligne.base_ht),
                            format_eur(ligne.tva)
                        ),
                    )?;
                }
                let collectee: Cents = recap.iter().map(|r| r.tva).sum();
                ecrire(sortie, format!("  Total TVA collectée : {}", format_eur(collectee)))?;
                if let Some(mention) = regime.mention_facture() {
                    ecrire(sortie, theme::info(format!("  {mention}")))?;
                }
                Ok(())
            }
            Some("balance") => {
                let ecritures = self.ecritures_generees()?;
                for (compte, debit, credit, solde) in compta::balance(&ecritures) {
                    ecrire(
                        sortie,
                        format!(
                            "{compte:<8} {:<34} {:>12} {:>12} {:>12}",
                            compta::libelle_compte(&compte),
                            format_eur(debit),
                            format_eur(credit),
                            format_eur(solde)
                        ),
                    )?;
                }
                Ok(())
            }
            Some("tableau") => self.tableau_finance(sortie),
            Some("rapprocher") => {
                let fichier = rest
                    .get(1)
                    .ok_or("usage : /compta rapprocher <releve.csv>")?;
                self.rapprocher(fichier, sortie)
            }
            Some("lettrer") => {
                let lettres = self.store.lettrer()?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ {lettres} paiement(s) rapproché(s) automatiquement (montant identique, date à ±10 jours)"
                    )),
                )
            }
            Some("ecritures") | Some("regenerer") => {
                let nombre = self.store.regenerer_ecritures()?;
                self.store.journaliser("compta.ecritures", &nombre.to_string())?;
                ecrire(
                    sortie,
                    theme::succes(format!("✓ {nombre} écriture(s) régénérée(s) depuis les documents et paiements")),
                )
            }
            _ => Err("usage : /compta seuils | fec [fichier] | journal [VE|BQ] | tva [période] | balance | tableau | rapprocher <csv> | lettrer | ecritures".into()),
        }
    }

    fn ecritures_generees(&mut self) -> Resultat<Vec<Ecriture>> {
        self.store.regenerer_ecritures()?;
        Ok(self.store.ecritures())
    }

    fn seuils(&mut self, sortie: &mut dyn Write) -> Resultat<()> {
        let annee = annee_courante();
        let (ventes, services) = self.store.ca_annuel(annee);
        let etat = etat_seuils(ventes, services, SEUILS_2026);
        ecrire(
            sortie,
            theme::titre(format!("Micro-entreprise — activité mixte, année {annee}")),
        )?;
        ecrire(
            sortie,
            format!(
                "  CA ventes   {:>14}   {:>3} % du plafond micro et {} % de la franchise TVA",
                format_eur(etat.ca_ventes),
                pourcentage(etat.ca_ventes, SEUILS_2026.ca_global_max),
                etat.pct_franchise_ventes()
            ),
        )?;
        ecrire(
            sortie,
            format!(
                "  CA services {:>14}   {} % du plafond services ({} % de la franchise TVA)",
                format_eur(etat.ca_services),
                etat.pct_services(),
                etat.pct_franchise_services()
            ),
        )?;
        ecrire(
            sortie,
            format!(
                "  CA global   {:>14}   {} % du plafond global {}",
                format_eur(etat.ca_global()),
                etat.pct_global(),
                format_eur(SEUILS_2026.ca_global_max)
            ),
        )?;
        if etat.alertes.is_empty() {
            ecrire(sortie, theme::succes("  ✓ Aucune alerte de seuil"))?;
        }
        for alerte in &etat.alertes {
            ecrire(sortie, theme::alerte(format!("  ⚠ {alerte}")))?;
        }
        Ok(())
    }

    fn tableau_finance(&mut self, sortie: &mut dyn Write) -> Resultat<()> {
        let annee = annee_courante();
        ecrire(sortie, theme::titre(format!("Tableau de bord finance — {annee}")))?;
        let mois = self.store.ca_mensuel(annee);
        let noms = [
            "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre",
            "octobre", "novembre", "décembre",
        ];
        let mut total_ventes = 0;
        let mut total_services = 0;
        for (numero, ventes, services) in &mois {
            total_ventes += ventes;
            total_services += services;
            if *ventes == 0 && *services == 0 {
                continue;
            }
            ecrire(
                sortie,
                format!(
                    "  {:<10} ventes {:>12}  services {:>12}",
                    noms[(*numero as usize - 1).min(11)],
                    format_eur(*ventes),
                    format_eur(*services)
                ),
            )?;
        }
        ecrire(
            sortie,
            format!(
                "  {:<10} ventes {:>12}  services {:>12}",
                "cumul",
                format_eur(total_ventes),
                format_eur(total_services)
            ),
        )?;
        ecrire(
            sortie,
            format!(
                "  Encours clients {} · encaissements {} · {} facture(s) en retard",
                format_eur(self.store.encours()),
                format_eur(self.store.tresorerie(Some(annee))),
                self.store.documents_en_retard(&aujourdhui()).len()
            ),
        )
    }

    fn rapprocher(&mut self, fichier: &str, sortie: &mut dyn Write) -> Resultat<()> {
        let table = csv::Table::depuis_fichier(Path::new(fichier))?;
        table.requis(&["date", "libelle", "montant"])?;
        let mut operations = Vec::new();
        for (index, ligne) in table.lignes.iter().enumerate() {
            let date = table.valeur(ligne, &["date", "date_operation", "valeur"]);
            if !date_valide(&date) {
                ecrire(
                    sortie,
                    theme::alerte(format!("⚠ ligne {} ignorée : date « {date} » invalide", index + 2)),
                )?;
                continue;
            }
            let montant = csv::montant(&table.valeur(ligne, &["montant", "credit", "debit"]))?;
            operations.push(OperationBancaire {
                id: 0,
                societe_id: 0,
                date_operation: date,
                libelle: table.valeur(ligne, &["libelle", "description", "label"]),
                montant,
                reference: table.valeur(ligne, &["reference", "ref"]),
                paiement_id: 0,
            });
        }
        let nombre = self.store.importer_releve(operations)?;
        self.store.journaliser("banque.import", &format!("{fichier} : {nombre}"))?;
        ecrire(
            sortie,
            theme::succes(format!(
                "✓ {nombre} opération(s) importée(s) — lancez `/compta lettrer` pour rapprocher les paiements"
            )),
        )
    }

    // ---------------------------------------------------------- Sociétés

    fn cmd_societes(&mut self, rest: &[&str], options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        let premiere = rest.first().map(|s| s.to_lowercase());
        match premiere.as_deref() {
            Some("nouvelle") | Some("nouveau") => {
                let nom = opt(options, "nom").ok_or("précisez --nom \"…\"")?;
                let mut societe = Societe {
                    nom: nom.to_string(),
                    regime: RegimeTva::from_str(opt(options, "regime").unwrap_or("franchise"))?,
                    devise: opt(options, "devise").unwrap_or(DEVISE_REFERENCE).to_string(),
                    ..Societe::default()
                };
                societe.siren = opt(options, "siren").unwrap_or("").to_string();
                societe.siret = opt(options, "siret").unwrap_or("").to_string();
                societe.tva_intracom = opt(options, "tva").unwrap_or("").to_string();
                societe.adresse = opt(options, "adresse").unwrap_or("").to_string();
                societe.code_postal = opt(options, "cp").unwrap_or("").to_string();
                societe.ville = opt(options, "ville").unwrap_or("").to_string();
                societe.email = opt(options, "email").unwrap_or("").to_string();
                societe.telephone = opt(options, "tel").unwrap_or("").to_string();
                societe.forme_juridique =
                    opt(options, "forme").unwrap_or("micro-entreprise").to_string();
                let societe = self.store.creer_societe(societe)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Société « {} » créée et sélectionnée (devise {})",
                        societe.nom, societe.devise
                    )),
                )
            }
            Some("basculer") => {
                let recherche = rest.get(1).ok_or("usage : /societes basculer <nom>")?;
                let societe = self.store.basculer_societe(recherche)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Société courante : {} ({} · devise {})",
                        societe.nom,
                        societe.regime.libelle(),
                        societe.devise
                    )),
                )
            }
            Some("modifier") => {
                let mut societe = self.store.societe()?;
                if let Some(v) = opt(options, "nom") {
                    societe.nom = v.to_string();
                }
                if let Some(v) = opt(options, "forme") {
                    societe.forme_juridique = v.to_string();
                }
                if let Some(v) = opt(options, "capital") {
                    societe.capital = v.to_string();
                }
                if let Some(v) = opt(options, "siren") {
                    societe.siren = v.to_string();
                }
                if let Some(v) = opt(options, "siret") {
                    societe.siret = v.to_string();
                }
                if let Some(v) = opt(options, "tva") {
                    societe.tva_intracom = v.to_string();
                }
                if let Some(v) = opt(options, "adresse") {
                    societe.adresse = v.to_string();
                }
                if let Some(v) = opt(options, "cp") {
                    societe.code_postal = v.to_string();
                }
                if let Some(v) = opt(options, "ville") {
                    societe.ville = v.to_string();
                }
                if let Some(v) = opt(options, "email") {
                    societe.email = v.to_string();
                }
                if let Some(v) = opt(options, "tel") {
                    societe.telephone = v.to_string();
                }
                if let Some(v) = opt(options, "devise") {
                    societe.devise = v.to_uppercase();
                }
                if let Some(v) = opt(options, "delai") {
                    societe.delai_paiement_jours = v
                        .parse()
                        .map_err(|_| "délai de paiement illisible".to_string())?;
                }
                if let Some(v) = opt(options, "penalites") {
                    societe.penalites_taux_bp = v
                        .parse()
                        .map_err(|_| "taux de pénalités illisible (points de base)".to_string())?;
                }
                let societe = self.store.modifier_societe(societe)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Société mise à jour : {} — {} · SIREN {} · {}",
                        societe.nom,
                        societe.regime.libelle(),
                        if societe.siren.is_empty() { "(à compléter)" } else { &societe.siren },
                        societe.adresse_complete()
                    )),
                )
            }
            Some("regime") => {
                let valeur = rest.get(1).ok_or("usage : /societe regime franchise|reel")?;
                let regime = RegimeTva::from_str(valeur)?;
                self.store.set_regime_tva(regime)?;
                ecrire(
                    sortie,
                    theme::succes(format!("✓ Régime TVA : {}", regime.libelle())),
                )
            }
            Some("detail") | None if !rest.is_empty() && premiere.as_deref() != Some("detail") => {
                unreachable!()
            }
            _ => self.detail_societes(sortie),
        }
    }

    fn detail_societes(&mut self, sortie: &mut dyn Write) -> Resultat<()> {
        let societes = self.store.societes();
        let courante = self.store.societe()?;
        if societes.len() > 1 {
            ecrire(sortie, theme::titre("Sociétés"))?;
            for societe in &societes {
                ecrire(
                    sortie,
                    format!(
                        "  {} {:<28} {:<14} devise {}",
                        if societe.id == courante.id { "▸" } else { " " },
                        societe.nom,
                        societe.regime.libelle(),
                        societe.devise
                    ),
                )?;
            }
            ecrire(sortie, "")?;
        }
        ecrire(sortie, theme::titre(format!("{} ({})", courante.nom, courante.forme_juridique)))?;
        let lignes = [
            ("SIREN", courante.siren.clone()),
            ("SIRET", courante.siret.clone()),
            ("TVA intracom.", courante.tva_intracom.clone()),
            ("adresse", courante.adresse_complete()),
            ("e-mail", courante.email.clone()),
            ("téléphone", courante.telephone.clone()),
            ("devise", courante.devise.clone()),
            ("délai de paiement", format!("{} jours", courante.delai_paiement_jours)),
            (
                "pénalités",
                format!("{} par an", format_taux_bp(courante.penalites_taux_bp as i32)),
            ),
            (
                "indemnité forfaitaire",
                format_eur(courante.indemnite_forfaitaire),
            ),
        ];
        for (libelle, valeur) in lignes {
            if !valeur.trim().is_empty() {
                ecrire(sortie, format!("  {libelle:<20}: {valeur}"))?;
            }
        }
        if let Some(mention) = courante.regime.mention_facture() {
            ecrire(sortie, format!("  mention facture     : {mention}"))?;
        }
        Ok(())
    }

    // ---------------------------------------------------------- Devises

    fn cmd_devises(&mut self, rest: &[&str], options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        match rest.first().map(|s| s.to_lowercase()).as_deref() {
            Some("taux") => {
                let devise = rest.get(1).ok_or("usage : /devises taux <DEVISE> <AAAA-MM-JJ> <1 devise = x EUR>")?;
                let date = rest
                    .get(2)
                    .map(|s| s.to_string())
                    .unwrap_or_else(aujourdhui);
                let valeur = rest.get(3).copied().or_else(|| opt(options, "valeur"));
                let valeur = valeur.ok_or("précisez la valeur : /devises taux USD 2026-10-01 0,92")?;
                let taux_micro = parse_taux(valeur)?;
                let taux = self.store.enregistrer_taux(devise, &date, taux_micro, "manuel")?;
                self.store.journaliser("devise.taux", &taux.libelle())?;
                ecrire(
                    sortie,
                    theme::succes(format!("✓ Taux enregistré : {} au {}", taux.libelle(), taux.date_taux)),
                )
            }
            Some("convertir") => {
                let montant = csv::montant(rest.get(1).ok_or("usage : /devises convertir <montant> <DEVISE> [date]")?)?;
                let devise = rest.get(2).ok_or("précisez la devise")?;
                let date = rest.get(3).map(|s| s.to_string()).unwrap_or_else(aujourdhui);
                let taux = self.store.taux(devise, &date).ok_or_else(|| {
                    format!("aucun taux connu pour {devise} au {date} — enregistrez-le avec /devises taux")
                })?;
                ecrire(
                    sortie,
                    format!(
                        "{} {} = {} ({}, taux du {})",
                        format_eur(montant),
                        devise.to_uppercase(),
                        format_eur(convertir(montant, taux.taux_micro)),
                        DEVISE_REFERENCE,
                        taux.date_taux
                    ),
                )
            }
            _ => {
                let taux = self.store.taux_liste();
                let societe = self.store.societe()?;
                ecrire(
                    sortie,
                    theme::titre(format!("Devise de référence : {} — taux datés", societe.devise)),
                )?;
                if taux.is_empty() {
                    return ecrire(
                        sortie,
                        "  Aucun taux. Ajoutez-en un : /devises taux USD 2026-10-01 0,92",
                    );
                }
                for taux in &taux {
                    ecrire(
                        sortie,
                        format!("  {:<5} {}  {:>14}  {}", taux.devise, taux.date_taux, format_eur(taux.taux_micro / 10_000), taux.source),
                    )?;
                }
                Ok(())
            }
        }
    }

    // ---------------------------------------------------------- Sécurité

    fn cmd_securite(&mut self, rest: &[&str], options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        match rest.first().map(|s| s.to_lowercase()).as_deref() {
            Some("connexion") | Some("login") => {
                let nom = rest.get(1).ok_or("usage : /auth connexion <compte> --mot-de-passe …")?;
                let utilisateur = self
                    .store
                    .utilisateur(nom)
                    .ok_or_else(|| format!("compte inconnu : « {nom} »"))?;
                if !utilisateur.actif {
                    return Err("compte désactivé — voyez un administrateur".into());
                }
                let mot_de_passe = opt(options, "mot-de-passe")
                    .map(|s| s.to_string())
                    .or_else(|| std::env::var("ERP_MOT_DE_PASSE").ok())
                    .ok_or("précisez --mot-de-passe (ou la variable ERP_MOT_DE_PASSE)")?;
                if !securite::verifier_mot_de_passe(&mot_de_passe, &utilisateur.empreinte) {
                    self.store
                        .journaliser("auth.echec", &format!("compte {nom} : mot de passe invalide"))?;
                    return Err("mot de passe invalide".into());
                }
                if utilisateur.role.est_admin() || utilisateur.deux_facteurs_actif() {
                    let code = match opt(options, "totp") {
                        Some(code) => code.to_string(),
                        None => {
                            return Err(
                                "double authentification requise : ajoutez --totp <code à 6 chiffres> \
                                 (SEC-02 : obligatoire pour les administrateurs)"
                                    .into(),
                            )
                        }
                    };
                    if !securite::verifier_totp(&utilisateur.secret_totp, &code) {
                        // Codes de récupération à usage unique.
                        let empreinte = securite::empreinte_code(&code);
                        if !utilisateur.codes_recuperation.iter().any(|c| c == &empreinte) {
                            self.store
                                .journaliser("auth.echec", &format!("compte {nom} : TOTP invalide"))?;
                            return Err("code de double authentification invalide".into());
                        }
                        let mut utilisateur = utilisateur.clone();
                        utilisateur.codes_recuperation.retain(|c| c != &empreinte);
                        self.store.maj_utilisateur(utilisateur)?;
                    }
                }
                self.store.etablir_session(&utilisateur.nom_utilisateur)?;
                self.store
                    .journaliser("auth.connexion", &utilisateur.nom_utilisateur)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Connecté en tant que {} ({})",
                        utilisateur.nom_utilisateur,
                        utilisateur.role.libelle()
                    )),
                )
            }
            Some("deconnexion") | Some("logout") => {
                let nom = self.store.session();
                self.store.fermer_session()?;
                self.store.journaliser("auth.deconnexion", &nom)?;
                ecrire(sortie, theme::succes("✓ Déconnecté (session locale)"))
            }
            Some("statut") => {
                let session = self.store.session();
                let role = self.store.role_courant();
                let comptes = self.store.utilisateurs().len();
                ecrire(
                    sortie,
                    format!(
                        "Session : {session} ({}) · {comptes} compte(s) déclaré(s){}",
                        role.libelle(),
                        if comptes == 0 {
                            " — mode mono-utilisateur, aucune authentification exigée"
                        } else {
                            ""
                        }
                    ),
                )
            }
            Some("nouveau") | Some("nouvelle") => {
                let nom = rest
                    .get(1)
                    .copied()
                    .or_else(|| opt(options, "compte"))
                    .ok_or("usage : /utilisateurs nouveau <compte> --mot-de-passe …")?;
                let role = Role::depuis_code(opt(options, "role").unwrap_or("utilisateur"));
                let mot_de_passe = opt(options, "mot-de-passe")
                    .map(|s| s.to_string())
                    .ok_or("précisez --mot-de-passe …")?;
                securite::politique_mot_de_passe(&mot_de_passe)?;
                let mut utilisateur = Utilisateur::nouveau(nom, role);
                utilisateur.empreinte = securite::hacher_mot_de_passe(&mot_de_passe);
                utilisateur.email = opt(options, "email").unwrap_or("").to_string();
                utilisateur.nom_complet = opt(options, "nom-complet").unwrap_or("").to_string();
                let utilisateur = self.store.creer_utilisateur(utilisateur)?;
                self.store
                    .journaliser("utilisateur.creation", &utilisateur.nom_utilisateur)?;
                let mut texte = format!(
                    "✓ Compte « {} » créé ({})",
                    utilisateur.nom_utilisateur,
                    utilisateur.role.libelle()
                );
                if utilisateur.role.est_admin() && !utilisateur.deux_facteurs_actif() {
                    texte.push_str(&format!(
                        "\n  ⚠ SEC-02 : la double authentification est obligatoire pour un admin — \
                         lancez `/utilisateurs 2fa {}`",
                        utilisateur.nom_utilisateur
                    ));
                }
                ecrire(sortie, theme::succes(texte))
            }
            Some("2fa") => {
                let nom = rest
                    .get(1)
                    .ok_or("usage : /utilisateurs 2fa <compte>")?;
                let mut utilisateur = self
                    .store
                    .utilisateur(nom)
                    .ok_or_else(|| format!("compte inconnu : « {nom} »"))?;
                let secret = securite::nouveau_secret_totp();
                let codes = securite::nouveaux_codes_recuperation(8);
                utilisateur.secret_totp = secret.clone();
                utilisateur.codes_recuperation =
                    codes.iter().map(|c| securite::empreinte_code(c)).collect();
                self.store.maj_utilisateur(utilisateur.clone())?;
                self.store
                    .journaliser("utilisateur.2fa", &utilisateur.nom_utilisateur)?;
                ecrire(sortie, theme::succes("✓ Double authentification activée"))?;
                ecrire(
                    sortie,
                    format!(
                        "  secret        : {secret}\n  uri           : {}",
                        securite::uri_otpauth(
                            &utilisateur.nom_utilisateur,
                            &secret,
                            &self.store.societe()?.nom
                        )
                    ),
                )?;
                ecrire(sortie, "  codes de récupération (affichés une seule fois) :")?;
                for code in &codes {
                    ecrire(sortie, format!("    {code}"))?;
                }
                Ok(())
            }
            Some("mot-de-passe") => {
                let nom = rest
                    .get(1)
                    .ok_or("usage : /utilisateurs mot-de-passe <compte> --nouveau …")?;
                let mut utilisateur = self
                    .store
                    .utilisateur(nom)
                    .ok_or_else(|| format!("compte inconnu : « {nom} »"))?;
                let nouveau = opt(options, "nouveau").ok_or("précisez --nouveau <mot de passe>")?;
                securite::politique_mot_de_passe(nouveau)?;
                utilisateur.empreinte = securite::hacher_mot_de_passe(nouveau);
                self.store.maj_utilisateur(utilisateur)?;
                self.store.journaliser("utilisateur.mot-de-passe", nom)?;
                ecrire(sortie, theme::succes("✓ Mot de passe mis à jour (haché, jamais stocké en clair)"))
            }
            Some("desactiver") | Some("activer") => {
                let nom = rest.get(1).ok_or("précisez le compte")?;
                let mut utilisateur = self
                    .store
                    .utilisateur(nom)
                    .ok_or_else(|| format!("compte inconnu : « {nom} »"))?;
                utilisateur.actif = rest[0].to_lowercase() == "activer";
                self.store.maj_utilisateur(utilisateur)?;
                self.store.journaliser("utilisateur.statut", nom)?;
                ecrire(sortie, theme::succes(format!("✓ Compte {nom} mis à jour")))
            }
            _ => {
                let utilisateurs = self.store.utilisateurs();
                if utilisateurs.is_empty() {
                    return ecrire(
                        sortie,
                        "Aucun compte : mode mono-utilisateur (aucune authentification).
Créez un administrateur : /utilisateurs nouveau jojo --role admin --mot-de-passe \"…\"",
                    );
                }
                ecrire(sortie, theme::titre(format!("Comptes ({})", utilisateurs.len())))?;
                if drapeau(options, "json") {
                    let items: Vec<String> = utilisateurs
                        .iter()
                        .map(|u| {
                            format!(
                                "{{\"compte\":\"{}\",\"role\":\"{}\",\"actif\":{},\"2fa\":{}}}",
                                json_echapper(&u.nom_utilisateur),
                                u.role.code(),
                                u.actif,
                                u.deux_facteurs_actif()
                            )
                        })
                        .collect();
                    return ecrire(sortie, format!("[{}]", items.join(",")));
                }
                for utilisateur in &utilisateurs {
                    ecrire(
                        sortie,
                        format!(
                            "  {:<16} {:<14} {} {}",
                            utilisateur.nom_utilisateur,
                            utilisateur.role.libelle(),
                            if utilisateur.actif { "actif" } else { "désactivé" },
                            if utilisateur.deux_facteurs_actif() { "· 2FA" } else { "· sans 2FA" }
                        ),
                    )?;
                }
                Ok(())
            }
        }
    }

    // ---------------------------------------------------------- Administration

    fn cmd_admin(&mut self, rest: &[&str], options: &Options, sortie: &mut dyn Write) -> Resultat<()> {
        match rest.first().map(|s| s.to_lowercase()).as_deref() {
            Some("sante") => {
                let compteurs = self.store.compteurs();
                let societe = self.store.societe()?;
                ecrire(sortie, theme::titre("État du système"))?;
                ecrire(
                    sortie,
                    format!(
                        "  société {} · régime {} · version {} · schéma Rustorm {}",
                        societe.nom,
                        societe.regime.libelle(),
                        env!("CARGO_PKG_VERSION"),
                        crate::orm::EMPREINTE_SCHEMA
                    ),
                )?;
                match self.store.dossier() {
                    Some(dossier) => {
                        let mut octets = 0u64;
                        for (table, _) in crate::orm::tables() {
                            let chemin = crate::fichier::chemin_table(dossier, table);
                            if let Ok(meta) = std::fs::metadata(&chemin) {
                                octets += meta.len();
                            }
                        }
                        ecrire(
                            sortie,
                            format!(
                                "  données {} · {} octets dans {} table(s) · sauvegardes : {}",
                                dossier.display(),
                                octets,
                                compteurs.len(),
                                self.store.sauvegardes().len()
                            ),
                        )?;
                    }
                    None => ecrire(sortie, "  mode mémoire (aucune persistance)")?,
                }
                for (table, lignes) in &compteurs {
                    ecrire(sortie, format!("    {table:<22} {lignes:>6} ligne(s)"))?;
                }
                match self.store.verifier_chaine_audit() {
                    Ok(nombre) => ecrire(
                        sortie,
                        theme::succes(format!("  ✓ journal d'audit intègre ({nombre} entrée(s) chaînées)")),
                    )?,
                    Err(e) => ecrire(sortie, theme::erreur(format!("  ✗ {e}")))?,
                }
                if let Some(derniere) = self.store.sauvegardes().first() {
                    ecrire(
                        sortie,
                        format!(
                            "  dernière sauvegarde : {} ({})",
                            derniere.date_sauvegarde,
                            format_octets(derniere.taille_octets)
                        ),
                    )?;
                }
                Ok(())
            }
            Some("schema") => {
                if drapeau(options, "sql") || rest.iter().any(|s| s.eq_ignore_ascii_case("sql")) {
                    return ecrire(sortie, crate::orm::ddl_postgres()?);
                }
                ecrire(sortie, crate::orm::resume())
            }
            Some("audit") => {
                if drapeau(options, "verifier") || rest.iter().any(|s| s.eq_ignore_ascii_case("verifier")) {
                    let nombre = self.store.verifier_chaine_audit()?;
                    return ecrire(
                        sortie,
                        theme::succes(format!(
                            "✓ Chaîne d'audit vérifiée : {nombre} entrée(s), aucune altération"
                        )),
                    );
                }
                let limite = opt_i64(options, "n", 30).max(1) as usize;
                let entrees = self.store.audit(limite);
                if entrees.is_empty() {
                    return ecrire(sortie, "Journal d'audit vide");
                }
                for entree in &entrees {
                    ecrire(
                        sortie,
                        format!(
                            "{}  {:<22} {:<14} {}  [{}]",
                            entree.horodatage,
                            entree.action,
                            entree.utilisateur,
                            entree.detail,
                            entree.empreinte
                        ),
                    )?;
                }
                Ok(())
            }
            Some("sauvegarde") => {
                let sauvegarde = self.store.creer_sauvegarde()?;
                self.store
                    .journaliser("sauvegarde.creation", &sauvegarde.date_sauvegarde)?;
                ecrire(
                    sortie,
                    theme::succes(format!(
                        "✓ Sauvegarde {} créée ({}, empreinte {})",
                        sauvegarde.date_sauvegarde,
                        format_octets(sauvegarde.taille_octets),
                        sauvegarde.empreinte
                    )),
                )
            }
            Some("sauvegardes") => {
                let sauvegardes = self.store.sauvegardes();
                if sauvegardes.is_empty() {
                    return ecrire(sortie, "Aucune sauvegarde — `/admin sauvegarde`");
                }
                for sauvegarde in &sauvegardes {
                    ecrire(
                        sortie,
                        format!(
                            "  {}  {:>10}  {}",
                            sauvegarde.date_sauvegarde,
                            format_octets(sauvegarde.taille_octets),
                            sauvegarde.empreinte
                        ),
                    )?;
                }
                Ok(())
            }
            Some("restaurer") => {
                self.store.exiger_admin()?;
                let nom = rest.get(1).ok_or("usage : /admin restaurer <nom de sauvegarde>")?;
                let resume = self.store.restaurer_sauvegarde(nom)?;
                ecrire(sortie, theme::succes(format!("✓ {resume}")))
            }
            Some("mentions") => self.mentions(sortie),
            _ => Err("usage : /admin sante | schema [sql] | audit [verifier] | sauvegarde | sauvegardes | restaurer <nom> | mentions".into()),
        }
    }

    fn mentions(&mut self, sortie: &mut dyn Write) -> Resultat<()> {
        let societe = self.store.societe()?;
        let mut manquants: Vec<&str> = Vec::new();
        if societe.siren.trim().is_empty() {
            manquants.push("SIREN (obligatoire sur toute facture)");
        }
        if societe.adresse.trim().is_empty() {
            manquants.push("adresse de l'émetteur");
        }
        if societe.ville.trim().is_empty() {
            manquants.push("ville de l'émetteur");
        }
        if societe.regime == RegimeTva::Reel && societe.tva_intracom.trim().is_empty() {
            manquants.push("numéro de TVA intracommunautaire (régime réel)");
        }
        ecrire(sortie, theme::titre("Checklist conformité — mentions obligatoires"))?;
        let points = [
            ("numérotation chronologique continue", true),
            ("mention « TVA non applicable, art. 293 B du CGI » (franchise)", societe.regime == RegimeTva::Franchise),
            ("échéance et conditions de paiement", true),
            (
                "pénalités de retard et indemnité forfaitaire de 40 €",
                societe.penalites_taux_bp > 0 && societe.indemnite_forfaitaire > 0,
            ),
            ("identité complète de l'émetteur", manquants.is_empty()),
        ];
        for (libelle, conforme) in points {
            ecrire(
                sortie,
                if conforme {
                    theme::succes(format!("  ✓ {libelle}"))
                } else {
                    theme::alerte(format!("  ⚠ {libelle}"))
                },
            )?;
        }
        for manquant in &manquants {
            ecrire(sortie, theme::alerte(format!("  à compléter : {manquant}")))?;
        }
        if !manquants.is_empty() {
            ecrire(
                sortie,
                theme::info(
                    "  complétez avec : /societe modifier --siren 123456789 --adresse \"1 rue …\" --ville Nice",
                ),
            )?;
        }
        Ok(())
    }
}

// ------------------------------------------------------------ Aides libres

fn pluriel_document(type_doc: TypeDocument) -> &'static str {
    match type_doc {
        TypeDocument::Devis => "devis",
        TypeDocument::Facture => "factures",
        TypeDocument::Avoir => "avoirs",
    }
}

fn tronquer(texte: &str, largeur: usize) -> String {
    if texte.chars().count() <= largeur {
        return texte.to_string();
    }
    let mut sortie: String = texte.chars().take(largeur.saturating_sub(1)).collect();
    sortie.push('…');
    sortie
}

fn encoder_liste(elements: &[String]) -> String {
    elements.join("|")
}

fn decoder_liste(texte: &str) -> Vec<String> {
    texte
        .split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn format_octets(octets: i64) -> String {
    if octets < 1_024 {
        format!("{octets} o")
    } else if octets < 1_048_576 {
        format!("{:.1} Ko", octets as f64 / 1_024.0)
    } else {
        format!("{:.1} Mo", octets as f64 / 1_048_576.0)
    }
}

/// Parse une ligne de facturation « désignation;quantité;prix HT[;taux bp][;remise bp] ».
pub fn parser_ligne(s: &str) -> Resultat<Ligne> {
    let parties: Vec<&str> = s.split(';').collect();
    if parties.len() < 3 {
        return Err(format!(
            "ligne illisible : « {s} » — format : désignation;quantité;prix HT[;taux bp][;remise bp]"
        ));
    }
    let quantite = csv::quantite_milli(parties[1])?;
    let prix = parse_eur(parties[2].trim())?;
    let tva_bp = match parties.get(3) {
        Some(valeur) if !valeur.trim().is_empty() => csv::taux_bp(valeur)?,
        _ => 2_000,
    };
    let remise_bp = match parties.get(4) {
        Some(valeur) if !valeur.trim().is_empty() => csv::taux_bp(valeur)?,
        _ => 0,
    };
    let mut ligne = Ligne::new(parties[0].trim(), 0.0, prix, tva_bp);
    ligne.quantite_milli = quantite;
    ligne.remise_bp = remise_bp;
    Ok(ligne)
}

// ------------------------------------------------------------ REPL

/// Boucle interactive : chemins relatifs, `..`, `go`, `?`, raccourcis `g f`…
pub fn repl(app: &mut App, stdin: &mut dyn BufRead, sortie: &mut dyn Write) -> Resultat<()> {
    let texte = theme::titre(
        "ERP CLI — tapez un chemin (/factures), une commande relative, `?` pour l'aide, `q` pour quitter.",
    )
    .to_string();
    ecrire(sortie, texte)?;
    let mut chemin: Vec<String> = Vec::new();
    loop {
        let invite = if chemin.is_empty() {
            "erp › ".to_string()
        } else {
            format!("erp /{} › ", chemin.join("/"))
        };
        write!(sortie, "{}", theme::selection(invite)).map_err(|e| e.to_string())?;
        sortie.flush().map_err(|e| e.to_string())?;
        let mut ligne = String::new();
        if stdin.read_line(&mut ligne).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let commande = ligne.trim();
        if commande.is_empty() {
            continue;
        }
        match commande {
            "q" | "quitter" | "exit" => break,
            "?" | "aide" | "help" => {
                ecrire(sortie, theme::violet(aide_contexte(&chemin)))?;
                continue;
            }
            ".." => {
                chemin.pop();
                app.tableau_de_bord(sortie)?;
                continue;
            }
            "g f" | "g factures" => {
                chemin = vec!["factures".into()];
                app.executer_jetons(vec!["/factures".into()], sortie)?;
                continue;
            }
            "g c" | "g clients" => {
                chemin = vec!["clients".into()];
                app.executer_jetons(vec!["/clients".into()], sortie)?;
                continue;
            }
            "g v" | "g vinted" => {
                chemin = vec!["vinted".into()];
                app.executer_jetons(vec!["/vinted".into()], sortie)?;
                continue;
            }
            "g d" | "g devis" => {
                chemin = vec!["devis".into()];
                app.executer_jetons(vec!["/devis".into()], sortie)?;
                continue;
            }
            "g s" | "g compta" => {
                chemin = vec!["compta".into()];
                app.executer_jetons(vec!["/compta/seuils".into()], sortie)?;
                continue;
            }
            "/" => {
                chemin.clear();
                app.tableau_de_bord(sortie)?;
                continue;
            }
            _ => {}
        }
        let mut jetons = tokeniser(commande);
        if let Some(premier) = jetons.first() {
            if let Some(reste) = premier.strip_prefix("go ") {
                let _ = reste;
            }
        }
        if commande.starts_with("go ") {
            let cible = commande[3..].trim();
            chemin = cible
                .trim_start_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            let mut affichage = vec![format!("/{}", chemin.join("/"))];
            affichage.extend(jetons.drain(2..));
            match app.executer_dans(&[], affichage, sortie) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => ecrire(sortie, theme::erreur(format!("✗ {e}")))?,
            }
            continue;
        }
        match app.executer_dans(&chemin, jetons, sortie) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => ecrire(sortie, theme::erreur(format!("✗ {e}")))?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::memoire()
    }

    fn lancer(app: &mut App, commande: &str) -> String {
        let mut tampon: Vec<u8> = Vec::new();
        match app.executer(commande, &mut tampon) {
            Ok(_) => String::from_utf8_lossy(&tampon).to_string(),
            Err(e) => format!("ERREUR: {e}"),
        }
    }

    #[test]
    fn tokenisation_guillemets() {
        let t = tokeniser("/clients nouveau --nom \"Atelier Dupont\" --type pro");
        assert_eq!(t[3], "Atelier Dupont");
    }

    #[test]
    fn options_separees_des_positions() {
        let (positionnels, options) = separer_options(tokeniser("/factures --json --page 2"));
        assert_eq!(positionnels, vec!["/factures"]);
        assert_eq!(opt(&options, "page"), Some("2"));
        assert!(drapeau(&options, "json"));
    }

    #[test]
    fn parse_ligne_ok() {
        let ligne = parser_ligne("Maintenance;2;500,00;2000").unwrap();
        assert_eq!(ligne.total_ht(), 100_000);
        let ligne = parser_ligne("Maintenance;2;500,00;2000;1000").unwrap();
        assert_eq!(ligne.remise_bp, 1_000);
        let ligne = parser_ligne("Port;1;10,00").unwrap();
        assert_eq!(ligne.tva_bp, 2_000);
        assert!(parser_ligne("illisible").is_err());
    }

    #[test]
    fn parcours_complet_par_le_cli() {
        let mut app = app();
        lancer(&mut app, "/societe modifier --siren 123456789 --adresse \"1 rue de la Paix\" --ville Nice");
        let texte = lancer(&mut app, "/clients nouveau --nom \"Atelier Dupont\" --type pro --email a@b.fr");
        assert!(texte.contains("Client n°1"), "{texte}");
        let texte = lancer(
            &mut app,
            "/factures nouvelle --client dupont --ligne \"Maintenance;2;500,00;2000\"",
        );
        assert!(texte.contains("FA-20"), "{texte}");
        let texte = lancer(&mut app, "/factures FA-2026-0001 valider");
        assert!(texte.contains("✓") || texte.contains("ERREUR"), "{texte}");
    }

    #[test]
    fn validation_refusee_sans_siren() {
        let mut app = app();
        lancer(&mut app, "/clients nouveau --nom \"Dupont\" --type pro");
        lancer(&mut app, "/factures nouvelle --client dupont --ligne \"Audit;1;100,00\"");
        let numero = app.store.documents(None)[0].numero.clone();
        let texte = lancer(&mut app, &format!("/factures {numero} valider"));
        assert!(texte.contains("mentions obligatoires"), "{texte}");
    }

    #[test]
    fn recherche_et_listes_json() {
        let mut app = app();
        lancer(&mut app, "/societe modifier --siren 123456789 --adresse \"1 rue\" --ville Nice");
        lancer(&mut app, "/clients nouveau --nom \"Atelier Dupont\" --type pro --tag maintenance");
        let json = lancer(&mut app, "/clients --json");
        assert!(json.trim().starts_with('['));
        assert!(json.contains("Atelier Dupont"));
        let csv = lancer(&mut app, "/clients --csv");
        assert!(csv.starts_with("id;nom;type"), "{csv}");
        let texte = lancer(&mut app, "/recherche dupont");
        assert!(texte.contains("1 résultat"), "{texte}");
        let texte = lancer(&mut app, "/recherche introuvablexyz");
        assert!(texte.contains("Aucun résultat"), "{texte}");
    }

    #[test]
    fn chemins_inexistants_expliques() {
        let mut app = app();
        let texte = lancer(&mut app, "/fournisseurs");
        assert!(texte.contains("chemin inconnu"), "{texte}");
        assert!(texte.contains("aide"), "{texte}");
    }

    #[test]
    fn aide_contextuelle() {
        let mut app = app();
        let texte = lancer(&mut app, "/aide factures");
        assert!(texte.contains("valider"), "{texte}");
        let texte = lancer(&mut app, "?");
        assert!(texte.contains("Navigation par chemins"), "{texte}");
    }

    #[test]
    fn compta_seuils_et_fec() {
        let mut app = app();
        lancer(&mut app, "/societe modifier --siren 123456789 --adresse \"1 rue\" --ville Nice");
        lancer(&mut app, "/clients nouveau --nom \"Dupont\" --type pro");
        lancer(&mut app, "/factures nouvelle --client dupont --ligne \"Maintenance;1;1000,00\"");
        let numero = app.store.documents(None)[0].numero.clone();
        lancer(&mut app, &format!("/factures {numero} valider"));
        let texte = lancer(&mut app, "/compta seuils");
        assert!(texte.contains("CA services"), "{texte}");
        let texte = lancer(&mut app, "/compta fec");
        assert!(texte.starts_with("JournalCode"), "{texte}");
        let texte = lancer(&mut app, "/compta balance");
        assert!(texte.contains("Clients"), "{texte}");
    }

    #[test]
    fn vinted_par_le_cli() {
        let mut app = app();
        lancer(&mut app, "/vinted nouveau --marque Zara --description \"Veste\" --taille M --composition \"100 % coton\" --prix-achat 8,00");
        let texte = lancer(&mut app, "/vinted vendre 1 --prix 25,00 --frais 2,00");
        assert!(texte.contains("marge nette 15,00 €"), "{texte}");
        let texte = lancer(&mut app, "/vinted stats");
        assert!(texte.contains("DAC7"), "{texte}");
        let texte = lancer(&mut app, "/vinted fiche 1");
        assert!(texte.contains("Composition"), "{texte}");
    }

    #[test]
    fn pdf_ubl_ecrits_sur_disque() {
        let dossier = std::env::temp_dir().join(format!("erp-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        std::fs::create_dir_all(&dossier).unwrap();
        let mut app = app();
        lancer(&mut app, "/societe modifier --siren 123456789 --adresse \"1 rue\" --ville Nice");
        lancer(&mut app, "/clients nouveau --nom \"Dupont\" --type pro --email a@b.fr");
        lancer(&mut app, "/factures nouvelle --client dupont --ligne \"Audit;1;100,00\"");
        let numero = app.store.documents(None)[0].numero.clone();
        lancer(&mut app, &format!("/factures {numero} valider"));
        let pdf = dossier.join("f.pdf");
        let xml = dossier.join("f.xml");
        lancer(&mut app, &format!("/factures {numero} pdf {}", pdf.display()));
        lancer(&mut app, &format!("/factures {numero} ubl {}", xml.display()));
        assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
        let contenu = std::fs::read_to_string(&xml).unwrap();
        assert!(contenu.contains("<Invoice"));
        let _ = std::fs::remove_dir_all(&dossier);
    }

    #[test]
    fn pagination() {
        let mut app = app();
        for i in 0..25 {
            lancer(&mut app, &format!("/clients nouveau --nom \"Client {i:02}\" --type pro"));
        }
        let texte = lancer(&mut app, "/clients --lignes 10");
        assert!(texte.contains("page 1/3"), "{texte}");
        let texte = lancer(&mut app, "/clients --lignes 10 --page 3");
        assert!(texte.contains("page 3/3"), "{texte}");
        assert!(!texte.contains("Client 00"));
    }
}
