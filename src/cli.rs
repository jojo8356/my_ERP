//! Interface CLI : navigation par chemins type URL (CDC §6), aucune GUI.
//! Exemples : `erp /clients`, `erp /factures/FA-2026-0001 payer`, `erp /compta/seuils`.

use crate::domain::*;
use crate::store::*;
use std::io::{BufRead, Write};

pub struct App {
    pub store: Box<dyn Store>,
}

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

/// Options `--cle valeur` ou `--cle=valeur` extraites des jetons.
fn options(jetons: &[String]) -> std::collections::HashMap<String, Vec<String>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut i = 0;
    while i < jetons.len() {
        let j = &jetons[i];
        if let Some(reste) = j.strip_prefix("--") {
            if let Some((k, v)) = reste.split_once('=') {
                map.entry(k.to_string()).or_default().push(v.to_string());
                i += 1;
            } else if i + 1 < jetons.len() {
                map.entry(reste.to_string())
                    .or_default()
                    .push(jetons[i + 1].clone());
                i += 2;
            } else {
                map.entry(reste.to_string()).or_default().push("oui".into());
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    map
}

fn opt<'a>(opts: &'a std::collections::HashMap<String, Vec<String>>, cle: &str) -> Option<&'a str> {
    opts.get(cle).and_then(|v| v.first()).map(|s| s.as_str())
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub const AIDE: &str = "\
Navigation par chemins (comme des URL, à connaître par cœur) :
  erp /                          tableau de bord (impayés, seuils, alertes)
  erp /clients                   liste des clients        (alias interactif : g c)
  erp /clients nouveau --nom \"Dupont\" --type pro --email a@b.fr
  erp /clients dupont            fiche client (recherche floue)
  erp /clients dupont factures   ses documents
  erp /devis                     liste des devis          (alias : g d)
  erp /factures                  liste des factures       (alias : g f)
  erp /factures nouvelle --client dupont --activite services --ligne \"Maintenance;2;500,00;2000\"
  erp /factures FA-2026-0001             détail d'une facture
  erp /factures FA-2026-0001 valider     brouillon -> validée (devient immuable)
  erp /factures FA-2026-0001 payer [montant] [AAAA-MM-JJ] [moyen]
  erp /factures FA-2026-0001 avoir       crée un avoir (corrige une facture validée)
  erp /devis DEV-2026-0001 transformer   devis -> facture
  erp /vinted                    articles + ventes        (alias : g v)
  erp /vinted nouveau --marque Zara --description \"Veste\" --taille M --composition \"100 % coton\" --prix-achat 8,00
  erp /vinted vendre 3 --prix 25,00 --frais 2,00
  erp /vinted stats              marges, DAC7
  erp /compta seuils             CA ventes/services vs plafonds micro 2026
  erp /compta fec [fichier]      export FEC (expert-comptable)
  erp /societe                   société, régime TVA  |  erp /societe regime franchise|reel
  erp /admin sante               état du système  |  erp /admin audit
Options utiles : --json sur les listes. Mode interactif : lancez `erp` seul (q pour quitter, ? = cette aide).
Interface plein écran dans le terminal : `erp tui` (Tab = module, ↑↓ = naviguer, v = valider, p = payer, q = quitter).
Format d'une ligne : \"désignation;quantité;prix unitaire HT;taux TVA en points de base (2000 = 20 %)\"";

impl App {
    pub fn executer(&mut self, commande_brute: &str, sortie: &mut dyn Write) -> Result<bool, String> {
        self.executer_jetons(tokeniser(commande_brute), sortie)
    }

    /// Comme `executer`, mais avec les arguments déjà découpés (préserve les
    /// valeurs multi-mots passées entre guillemets par le shell).
    pub fn executer_jetons(&mut self, mut jetons: Vec<String>, sortie: &mut dyn Write) -> Result<bool, String> {
        if jetons.is_empty() {
            return self.tableau_de_bord(sortie).map(|_| true);
        }
        // Chemin : on retire les "/" de tête et on éclate les segments restants.
        let premier = jetons.remove(0);
        let mut chemin: Vec<String> = premier
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        chemin.extend(jetons);
        let chemin_ref: Vec<&str> = chemin.iter().map(|s| s.as_str()).collect();
        self.journaliser_action(&chemin_ref);
        match chemin_ref.as_slice() {
            [] => self.tableau_de_bord(sortie)?,
            ["aide"] | ["?"] | ["help"] => writeln!(sortie, "{AIDE}").map_err(|e| e.to_string())?,
            ["quitter"] | ["q"] | ["exit"] => return Ok(false),
            ["clients", rest @ ..] => self.cmd_clients(rest, sortie)?,
            ["devis", rest @ ..] => self.cmd_documents(TypeDocument::Devis, rest, sortie)?,
            ["factures", rest @ ..] => self.cmd_documents(TypeDocument::Facture, rest, sortie)?,
            ["vinted", rest @ ..] => self.cmd_vinted(rest, sortie)?,
            ["compta", rest @ ..] => self.cmd_compta(rest, sortie)?,
            ["societe", rest @ ..] => self.cmd_societe(rest, sortie)?,
            ["admin", rest @ ..] => self.cmd_admin(rest, sortie)?,
            _ => {
                return Err(format!(
                    "chemin inconnu : « /{} » — tapez `aide` pour la liste des chemins",
                    chemin_ref.join("/")
                ))
            }
        }
        Ok(true)
    }

    fn journaliser_action(&mut self, chemin: &[&str]) {
        let _ = self.store.journaliser("cli", &format!("/{}", chemin.join("/")));
    }

    fn tableau_de_bord(&mut self, sortie: &mut dyn Write) -> Result<(), String> {
        let societe = self.store.societe()?;
        let annee = annee_de(&aujourdhui());
        let (ventes, services) = self.store.ca_annuel(annee)?;
        let etat = etat_seuils(ventes, services, SEUILS_2026);
        let docs = self.store.documents(Some(TypeDocument::Facture))?;
        let impayes: Vec<&Document> = docs
            .iter()
            .filter(|d| matches!(d.statut, StatutDocument::Valide | StatutDocument::PartiellementPaye))
            .collect();
        writeln!(sortie, "┌─ {} — tableau de bord {}", societe.nom, annee).map_err(|e| e.to_string())?;
        writeln!(sortie, "│ CA ventes : {} · CA services : {}", format_eur(ventes), format_eur(services)).map_err(|e| e.to_string())?;
        writeln!(sortie, "│ Factures en attente de paiement : {}", impayes.len()).map_err(|e| e.to_string())?;
        for d in &impayes {
            let reste = reste_a_payer(d.totaux(societe.regime).ttc, d.paye);
            writeln!(sortie, "│   ! {} — reste {}", d.numero, format_eur(reste)).map_err(|e| e.to_string())?;
        }
        for a in &etat.alertes {
            writeln!(sortie, "│ ⚠ {a}").map_err(|e| e.to_string())?;
        }
        if let Some(a) = self.alertes_dac7(annee)? {
            writeln!(sortie, "│ ⚠ {a}").map_err(|e| e.to_string())?;
        }
        writeln!(sortie, "└─ Tapez un chemin (/clients, /factures, /vinted, /compta/seuils) ou `aide`").map_err(|e| e.to_string())?;
        Ok(())
    }

    fn alertes_dac7(&mut self, annee: u32) -> Resultat<Option<String>> {
        let ventes: Vec<Vente> = self
            .store
            .ventes()?
            .into_iter()
            .filter(|v| annee_de(&v.date_vente) == annee)
            .collect();
        let ca: Cents = ventes.iter().map(|v| v.prix_vente).sum();
        Ok(alerte_dac7(ventes.len() as u32, ca))
    }

    // ------------------------------------------------------------ Clients
    fn cmd_clients(&mut self, rest: &[&str], sortie: &mut dyn Write) -> Result<(), String> {
        match rest {
            [] | ["--json"] => {
                let clients = self.store.clients()?;
                if rest.first() == Some(&"--json") {
                    let items: Vec<String> = clients
                        .iter()
                        .map(|c| format!("{{\"id\":{},\"nom\":\"{}\",\"type\":\"{}\",\"email\":\"{}\"}}", c.id, json_escape(&c.nom), c.type_client.as_str(), json_escape(&c.email)))
                        .collect();
                    writeln!(sortie, "[{}]", items.join(",")).map_err(|e| e.to_string())?;
                    return Ok(());
                }
                if clients.is_empty() {
                    writeln!(sortie, "Aucun client. Créez-en un : /clients nouveau --nom \"…\" --type pro").map_err(|e| e.to_string())?;
                }
                for c in &clients {
                    writeln!(sortie, "{:>4}  {} — {} — {}", c.id, c.nom, c.type_client.as_str(), c.email).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            ["nouveau", args @ ..] => {
                let jetons: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                let o = options(&jetons);
                let c = self.store.creer_client(Client {
                    id: 0,
                    nom: opt(&o, "nom").unwrap_or("").to_string(),
                    type_client: TypeClient::from_str(opt(&o, "type").unwrap_or("pro"))?,
                    email: opt(&o, "email").unwrap_or("").to_string(),
                    telephone: opt(&o, "tel").unwrap_or("").to_string(),
                    adresse: opt(&o, "adresse").unwrap_or("").to_string(),
                    siren: opt(&o, "siren").unwrap_or("").to_string(),
                    tva_intracom: opt(&o, "tva").unwrap_or("").to_string(),
                    notes: opt(&o, "notes").unwrap_or("").to_string(),
                })?;
                writeln!(sortie, "✓ Client n°{} créé : {}", c.id, c.nom).map_err(|e| e.to_string())?;
                Ok(())
            }
            [recherche, "factures"] => {
                let trouves = self.store.chercher_clients(recherche)?;
                let c = trouves.first().ok_or_else(|| format!("aucun client ne correspond à « {recherche} »"))?;
                writeln!(sortie, "Documents de {} :", c.nom).map_err(|e| e.to_string())?;
                for d in self.store.documents(None)? {
                    if d.client_id == c.id {
                        self.afficher_document_court(&d, sortie)?;
                    }
                }
                Ok(())
            }
            [recherche] => {
                let trouves = self.store.chercher_clients(recherche)?;
                match trouves.first() {
                    None => Err(format!("aucun client ne correspond à « {recherche} »")),
                    Some(c) => {
                        writeln!(sortie, "{} (n°{}, {})", c.nom, c.id, c.type_client.as_str()).map_err(|e| e.to_string())?;
                        if !c.email.is_empty() {
                            writeln!(sortie, "  e-mail : {}", c.email).map_err(|e| e.to_string())?;
                        }
                        if !c.siren.is_empty() {
                            writeln!(sortie, "  SIREN : {}", c.siren).map_err(|e| e.to_string())?;
                        }
                        if !c.adresse.is_empty() {
                            writeln!(sortie, "  adresse : {}", c.adresse).map_err(|e| e.to_string())?;
                        }
                        Ok(())
                    }
                }
            }
            _ => Err("usage : /clients [nouveau --nom … | <recherche> [factures]]".into()),
        }
    }

    // ------------------------------------------------------------ Documents
    fn afficher_document_court(&self, d: &Document, sortie: &mut dyn Write) -> Result<(), String> {
        // Le régime est relu par les appelants via les totaux affichés en détail ;
        // ici on affiche HT/TTC franchise par défaut si paye inconnu — voir détail.
        writeln!(
            sortie,
            "{} {}  {}  {}  payé : {}",
            d.statut.symbole(),
            d.numero,
            d.date_doc,
            d.statut.as_str(),
            format_eur(d.paye)
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn cmd_documents(&mut self, type_doc: TypeDocument, rest: &[&str], sortie: &mut dyn Write) -> Result<(), String> {
        let base = type_doc.as_str();
        match rest {
            [] | ["--json"] => {
                let docs = self.store.documents(Some(type_doc))?;
                let regime = self.store.societe()?.regime;
                if docs.is_empty() {
                    writeln!(sortie, "Aucun document. Créez-en un : /{base} nouvelle --client <recherche> --ligne \"…\"").map_err(|e| e.to_string())?;
                }
                for d in &docs {
                    let t = d.totaux(regime);
                    writeln!(
                        sortie,
                        "{} {}  {}  {:<22} TTC {}  ({})",
                        d.statut.symbole(),
                        d.numero,
                        d.date_doc,
                        "",
                        format_eur(t.ttc),
                        d.statut.as_str()
                    )
                    .map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            ["nouvelle", args @ ..] | ["nouveau", args @ ..] => {
                let jetons: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                let o = options(&jetons);
                let recherche_client = opt(&o, "client").ok_or("précisez --client <nom ou e-mail>")?;
                let client = self
                    .store
                    .chercher_clients(recherche_client)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| format!("client « {recherche_client} » introuvable"))?;
                let mut lignes = Vec::new();
                for l in o.get("ligne").cloned().unwrap_or_default() {
                    lignes.push(parser_ligne(&l)?);
                }
                if lignes.is_empty() {
                    return Err("ajoutez au moins une --ligne \"désignation;quantité;prix HT;taux bp\"".into());
                }
                let date = opt(&o, "date").unwrap_or(&aujourdhui()).to_string();
                let doc = self.store.creer_document(
                    type_doc,
                    client.id,
                    &date,
                    opt(&o, "echeance").unwrap_or(""),
                    opt(&o, "activite").unwrap_or("services"),
                    lignes,
                )?;
                self.store.journaliser("document.création", &doc.numero)?;
                writeln!(sortie, "✓ {} créé : {} (brouillon) — validez avec : /{base} {} valider", type_doc.as_str(), doc.numero, doc.numero).map_err(|e| e.to_string())?;
                Ok(())
            }
            [numero] => {
                let d = self.store.document_par_numero(numero)?;
                let regime = self.store.societe()?.regime;
                let client = self.store.client_par_id(d.client_id)?;
                let t = d.totaux(regime);
                writeln!(sortie, "{} {} — {} — client : {}", d.type_doc.as_str(), d.numero, d.statut.as_str(), client.nom).map_err(|e| e.to_string())?;
                writeln!(sortie, "  date : {} · échéance : {} · activité : {}", d.date_doc, d.echeance, d.activite).map_err(|e| e.to_string())?;
                for l in &d.lignes {
                    writeln!(sortie, "  - {} × {} à {} = {}", l.designation, l.quantite_milli as f64 / 1000.0, format_eur(l.prix_unitaire_ht), format_eur(l.total_ht())).map_err(|e| e.to_string())?;
                }
                writeln!(sortie, "  HT {} · TVA {} · TTC {} · payé {} · reste {}", format_eur(t.ht), format_eur(t.tva), format_eur(t.ttc), format_eur(d.paye), format_eur(reste_a_payer(t.ttc, d.paye))).map_err(|e| e.to_string())?;
                if let Some(m) = regime.mention_facture() {
                    writeln!(sortie, "  Mention : {m}").map_err(|e| e.to_string())?;
                }
                for p in self.store.paiements(d.id)? {
                    writeln!(sortie, "  paiement : {} le {} ({})", format_eur(p.montant), p.date_paiement, p.moyen).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            [numero, "valider"] => {
                let d = self.store.document_par_numero(numero)?;
                if !peut_modifier(d.statut) {
                    return Err(format!("{} est déjà {} : un document validé ne se modifie plus (créez un avoir)", numero, d.statut.as_str()));
                }
                let d = self.store.changer_statut(numero, StatutDocument::Valide)?;
                self.store.journaliser("document.validation", numero)?;
                writeln!(sortie, "✓ {} validée (figée, numérotation officielle)", d.numero).map_err(|e| e.to_string())?;
                Ok(())
            }
            [numero, "transformer"] if type_doc == TypeDocument::Devis => {
                let d = self.store.document_par_numero(numero)?;
                let f = self.store.creer_document(
                    TypeDocument::Facture,
                    d.client_id,
                    &aujourdhui(),
                    &d.echeance,
                    &d.activite,
                    d.lignes.clone(),
                )?;
                self.store.changer_statut(numero, StatutDocument::Accepte)?;
                self.store.journaliser("devis.transformation", &format!("{numero} -> {}", f.numero))?;
                writeln!(sortie, "✓ Devis {} transformé en facture {} (brouillon)", numero, f.numero).map_err(|e| e.to_string())?;
                Ok(())
            }
            [numero, "payer", args @ ..] => {
                let d = self.store.document_par_numero(numero)?;
                let regime = self.store.societe()?.regime;
                let montant = match args.first() {
                    Some(m) => parse_eur(m)?,
                    None => reste_a_payer(d.totaux(regime).ttc, d.paye),
                };
                if montant <= 0 {
                    return Err("montant du paiement nul ou déjà soldé".into());
                }
                let date = args.get(1).map(|s| s.to_string()).unwrap_or_else(aujourdhui);
                let moyen = args.get(2).map(|s| s.to_string()).unwrap_or_else(|| "virement".to_string());
                let d = self.store.ajouter_paiement(numero, montant, &date, &moyen)?;
                self.store.journaliser("paiement", &format!("{numero} {}", format_eur(montant)))?;
                writeln!(sortie, "✓ Paiement enregistré sur {} — statut : {} (reste {})", d.numero, d.statut.as_str(), format_eur(reste_a_payer(d.totaux(regime).ttc, d.paye))).map_err(|e| e.to_string())?;
                Ok(())
            }
            [numero, "avoir"] => {
                let d = self.store.document_par_numero(numero)?;
                if d.type_doc != TypeDocument::Facture {
                    return Err("un avoir ne corrige qu'une facture".into());
                }
                let lignes: Vec<Ligne> = d
                    .lignes
                    .iter()
                    .map(|l| Ligne {
                        designation: format!("Avoir sur {} — {}", d.numero, l.designation),
                        quantite_milli: l.quantite_milli,
                        prix_unitaire_ht: -l.prix_unitaire_ht,
                        tva_bp: l.tva_bp,
                    })
                    .collect();
                let a = self.store.creer_document(TypeDocument::Avoir, d.client_id, &aujourdhui(), "", &d.activite, lignes)?;
                self.store.journaliser("avoir.création", &format!("{} pour {}", a.numero, numero))?;
                writeln!(sortie, "✓ Avoir {} créé (brouillon) pour corriger {}", a.numero, numero).map_err(|e| e.to_string())?;
                Ok(())
            }
            [numero, action] => Err(format!(
                "action « {action} » inconnue pour {numero} — essayez : valider | payer | avoir{}",
                if type_doc == TypeDocument::Devis { " | transformer" } else { "" }
            )),
            _ => Err(format!("usage : /{base} [nouvelle --client … --ligne … | <numéro> [valider|payer|avoir]]")),
        }
    }

    // ------------------------------------------------------------ Vinted
    fn cmd_vinted(&mut self, rest: &[&str], sortie: &mut dyn Write) -> Result<(), String> {
        match rest {
            [] => {
                let articles = self.store.articles()?;
                if articles.is_empty() {
                    writeln!(sortie, "Aucun article. Ajoutez-en un : /vinted nouveau --marque … --description … --composition \"…\" --prix-achat 8,00").map_err(|e| e.to_string())?;
                }
                for a in &articles {
                    writeln!(sortie, "{:>4}  {:<14} {:<28} taille {:<4} achat {:>10}  [{}]", a.id, a.marque, a.description, a.taille, format_eur(a.prix_achat), a.statut).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            ["nouveau", args @ ..] => {
                let jetons: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                let o = options(&jetons);
                let a = self.store.creer_article(Article {
                    id: 0,
                    marque: opt(&o, "marque").unwrap_or("sans marque").to_string(),
                    description: opt(&o, "description").unwrap_or("").to_string(),
                    taille: opt(&o, "taille").unwrap_or("").to_string(),
                    composition: opt(&o, "composition").unwrap_or("").to_string(),
                    etat: opt(&o, "etat").unwrap_or("très bon").to_string(),
                    prix_achat: parse_eur(opt(&o, "prix-achat").unwrap_or("0"))?,
                    date_achat: opt(&o, "date").unwrap_or(&aujourdhui()).to_string(),
                    statut: "en_stock".to_string(),
                })?;
                writeln!(sortie, "✓ Article n°{} ajouté : {} {} (achat {}) — composition : {}", a.id, a.marque, a.description, format_eur(a.prix_achat), a.composition).map_err(|e| e.to_string())?;
                Ok(())
            }
            ["vendre", id, args @ ..] => {
                let jetons: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                let o = options(&jetons);
                let article_id: i64 = id.parse().map_err(|_| format!("identifiant d'article illisible : « {id} »"))?;
                let v = self.store.enregistrer_vente(Vente {
                    id: 0,
                    article_id,
                    date_vente: opt(&o, "date").unwrap_or(&aujourdhui()).to_string(),
                    prix_vente: parse_eur(opt(&o, "prix").unwrap_or("0"))?,
                    frais: parse_eur(opt(&o, "frais").unwrap_or("0"))?,
                    plateforme: opt(&o, "plateforme").unwrap_or("Vinted").to_string(),
                })?;
                let a = self.store.article_par_id(article_id)?;
                writeln!(sortie, "✓ Vente enregistrée : {} {} vendu {} — marge nette {}", a.marque, a.description, format_eur(v.prix_vente), format_eur(marge_nette(&a, &v))).map_err(|e| e.to_string())?;
                Ok(())
            }
            ["stats"] => {
                let annee = annee_de(&aujourdhui());
                let ventes = self.store.ventes()?;
                let mut total_marge: Cents = 0;
                let mut nb = 0u32;
                let mut ca: Cents = 0;
                for v in &ventes {
                    let a = self.store.article_par_id(v.article_id)?;
                    total_marge += marge_nette(&a, v);
                    if annee_de(&v.date_vente) == annee {
                        nb += 1;
                        ca += v.prix_vente;
                    }
                }
                writeln!(sortie, "Ventes totales : {} · marge nette cumulée : {}", ventes.len(), format_eur(total_marge)).map_err(|e| e.to_string())?;
                writeln!(sortie, "Année {annee} : {nb} ventes, CA {}", format_eur(ca)).map_err(|e| e.to_string())?;
                if let Some(a) = alerte_dac7(nb, ca) {
                    writeln!(sortie, "⚠ {a}").map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            _ => Err("usage : /vinted [nouveau --marque … | vendre <n°> --prix … | stats]".into()),
        }
    }

    // ------------------------------------------------------------ Comptabilité
    fn cmd_compta(&mut self, rest: &[&str], sortie: &mut dyn Write) -> Result<(), String> {
        match rest {
            ["seuils"] => {
                let annee = annee_de(&aujourdhui());
                let (ventes, services) = self.store.ca_annuel(annee)?;
                let e = etat_seuils(ventes, services, SEUILS_2026);
                writeln!(sortie, "Micro-entreprise — activité mixte, année {annee}").map_err(|e| e.to_string())?;
                writeln!(sortie, "  CA ventes   : {}  (franchise TVA dès {})", format_eur(e.ca_ventes), format_eur(SEUILS_2026.franchise_tva_ventes)).map_err(|e| e.to_string())?;
                writeln!(sortie, "  CA services : {}  (plafond micro services {})", format_eur(e.ca_services), format_eur(SEUILS_2026.ca_services_max)).map_err(|e| e.to_string())?;
                writeln!(sortie, "  CA global   : {}  (plafond micro global {})", format_eur(e.ca_ventes + e.ca_services), format_eur(SEUILS_2026.ca_global_max)).map_err(|e| e.to_string())?;
                if e.alertes.is_empty() {
                    writeln!(sortie, "  ✓ Aucune alerte de seuil").map_err(|e| e.to_string())?;
                }
                for a in &e.alertes {
                    writeln!(sortie, "  ⚠ {a}").map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            ["fec"] | ["fec", ..] => {
                // FEC simplifié : écritures dérivées des paiements (journal banque / ventes).
                let regime = self.store.societe()?.regime;
                let mut lignes = vec![
                    "JournalCode|JournalLib|EcritureNum|EcritureDate|CompteNum|CompteLib|CompAuxNum|CompAuxLib|PieceRef|PieceDate|EcritureLib|Debit|Credit|EcritureLet|DateLet|ValidDate|Montantdevise|Idevise"
                        .to_string(),
                ];
                for d in self.store.documents(Some(TypeDocument::Facture))? {
                    let client = self.store.client_par_id(d.client_id)?;
                    for p in self.store.paiements(d.id)? {
                        let m = format!("{},{:02}", p.montant.abs() / 100, p.montant.abs() % 100);
                        let date_compacte = p.date_paiement.replace('-', "");
                        lignes.push(format!(
                            "BQ|Banque|{}|{}|512000|Banque|||{}|{}|Paiement {}|{}|0,00|||||EUR",
                            d.numero, date_compacte, d.numero, date_compacte, client.nom, m
                        ));
                        lignes.push(format!(
                            "BQ|Banque|{}|{}|411000|Clients|{}|{}|{}|{}|Paiement {}|0,00|{}|||||EUR",
                            d.numero, date_compacte, client.id, client.nom, d.numero, date_compacte, client.nom, m
                        ));
                    }
                    let _ = regime;
                }
                match rest.get(1) {
                    Some(fichier) => {
                        std::fs::write(fichier, lignes.join("\n") + "\n").map_err(|e| format!("écriture du FEC : {e}"))?;
                        writeln!(sortie, "✓ FEC écrit dans {fichier} ({} lignes) — à faire valider par l'expert-comptable", lignes.len()).map_err(|e| e.to_string())?;
                    }
                    None => {
                        for l in &lignes {
                            writeln!(sortie, "{l}").map_err(|e| e.to_string())?;
                        }
                    }
                }
                Ok(())
            }
            _ => Err("usage : /compta seuils | /compta fec [fichier]".into()),
        }
    }

    // ------------------------------------------------------------ Société / admin
    fn cmd_societe(&mut self, rest: &[&str], sortie: &mut dyn Write) -> Result<(), String> {
        match rest {
            [] => {
                let s = self.store.societe()?;
                writeln!(sortie, "{} — régime TVA : {} — devise : {}", s.nom, s.regime.as_str(), s.devise).map_err(|e| e.to_string())?;
                if let Some(m) = s.regime.mention_facture() {
                    writeln!(sortie, "Mention sur les factures : {m}").map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            ["regime", r] => {
                let regime = RegimeTva::from_str(r)?;
                self.store.set_regime_tva(regime)?;
                writeln!(sortie, "✓ Régime TVA : {}", regime.as_str()).map_err(|e| e.to_string())?;
                Ok(())
            }
            _ => Err("usage : /societe [regime franchise|reel]".into()),
        }
    }

    fn cmd_admin(&mut self, rest: &[&str], sortie: &mut dyn Write) -> Result<(), String> {
        match rest {
            ["sante"] => {
                let nb_clients = self.store.clients()?.len();
                let nb_docs = self.store.documents(None)?.len();
                let nb_articles = self.store.articles()?.len();
                writeln!(sortie, "✓ Système sain — {nb_clients} clients, {nb_docs} documents, {nb_articles} articles").map_err(|e| e.to_string())?;
                Ok(())
            }
            ["audit"] => {
                let entrees = self.store.audit(30)?;
                if entrees.is_empty() {
                    writeln!(sortie, "Journal d'audit vide").map_err(|e| e.to_string())?;
                }
                for (d, a, detail) in &entrees {
                    writeln!(sortie, "{d}  {a:<24} {detail}").map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            _ => Err("usage : /admin sante | /admin audit".into()),
        }
    }
}

pub fn parser_ligne(s: &str) -> Result<Ligne, String> {
    let parts: Vec<&str> = s.split(';').collect();
    if parts.len() < 3 {
        return Err(format!("ligne illisible : « {s} » — format : désignation;quantité;prix HT[;taux bp]"));
    }
    let quantite: f64 = parts[1]
        .trim()
        .replace(',', ".")
        .parse()
        .map_err(|_| format!("quantité illisible dans « {s} »"))?;
    let prix = parse_eur(parts[2].trim())?;
    let tva_bp: i32 = parts
        .get(3)
        .map(|t| t.trim().parse().map_err(|_| format!("taux illisible dans « {s} »")))
        .transpose()?
        .unwrap_or(2000);
    Ok(Ligne::new(parts[0].trim(), quantite, prix, tva_bp))
}

/// Boucle interactive (REPL) : prompts, raccourcis `g f`, `g c`, `g v`, `g d`, `?`, `q`.
pub fn repl(app: &mut App, stdin: &mut dyn BufRead, sortie: &mut dyn Write) -> Result<(), String> {
    writeln!(sortie, "ERP CLI — tapez un chemin (/factures), un raccourci (g f) ou `aide`. `q` pour quitter.").map_err(|e| e.to_string())?;
    let _ = app.tableau_de_bord(sortie);
    loop {
        write!(sortie, "erp › ").map_err(|e| e.to_string())?;
        sortie.flush().map_err(|e| e.to_string())?;
        let mut ligne = String::new();
        if stdin.read_line(&mut ligne).map_err(|e| e.to_string())? == 0 {
            break; // EOF
        }
        let commande = match ligne.trim() {
            "g f" => "/factures".to_string(),
            "g c" => "/clients".to_string(),
            "g v" => "/vinted".to_string(),
            "g d" => "/devis".to_string(),
            "g s" => "/compta seuils".to_string(),
            autre => autre.to_string(),
        };
        match app.executer(&commande, sortie) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => writeln!(sortie, "✗ {e}").map_err(|err| err.to_string())?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenisation_guillemets() {
        let t = tokeniser("/clients nouveau --nom \"Atelier Dupont\" --type pro");
        assert_eq!(t[3], "Atelier Dupont");
    }

    #[test]
    fn parse_ligne_ok() {
        let l = parser_ligne("Maintenance;2;500,00;2000").unwrap();
        assert_eq!(l.total_ht(), 100_000);
    }
}
