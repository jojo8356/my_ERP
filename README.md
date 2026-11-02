# my_ERP

ERP sur mesure — le projet démarre par le **choix de la stack**, avec la méthode des grandes entreprises
(matrice de décision pondérée : besoin métier, scalabilité, équipe/recrutement, sécurité, TCO).

## 🗳️ Première étape : répondez au questionnaire de décision

Ouvrez **`index.html`** (artefact interactif) :

- 13 sections adaptées à un ERP : contexte, utilisateurs, modules, architecture, frontend, backend,
  données, sécurité/RGPD, intégrations, hébergement/DevOps, équipe/budget, arbitrages.
- 💾 Sauvegarde automatique dans le navigateur, reprise à tout moment.
- 🧭 Recommandation de stack générée en direct à partir de vos réponses.
- 📤 Export **JSON** (`mes-choix.json`) ou **Markdown** à transmettre à l'assistant pour enregistrement.

Version texte : [`CHOIX-STACK-ERP.md`](CHOIX-STACK-ERP.md) · Vos réponses versionnées : [`decisions/`](decisions/)

## 📄 Cahier des charges

**[CDC-ERP-CLI.md](CDC-ERP-CLI.md) · **lecture confort : [CDC.html](CDC.html)** (ouvrir dans un navigateur : sommaire, recherche, mode sombre)** — CDC complet v1.0 (ERP CLI Rust + PostgreSQL, conformité services informatiques & Vinted), en attente de validation (réponses au §20 + GO J0).

## Servir l'artefact en local

```bash
python3 -m http.server 8000
# puis http://localhost:8000
```

## 🖥️ V1 — ERP CLI en Rust (livrée, CI verte)

Zéro GUI. Couleurs : [Chromat](https://github.com/jojo8356/chromat)
(bibliothèque de l’auteur, ADR-004) pilote le CLI via `src/theme.rs` ;
la TUI utilise **Ratatui + Crossterm** (ADR-005) avec la même palette,
et des **formulaires interactifs** pour chaque création/action.
Navigation par **chemins type URL** et raccourcis,
comme exigé dans le [CDC](CDC-ERP-CLI.md). Modules v1 : CRM, devis/factures/avoirs,
comptabilité (seuils micro, FEC), achat-revente Vinted (marges, alertes DAC7).

```bash
cargo build --release            # binaire : target/release/my-erp
./target/release/my-erp          # mode interactif (erp ›), tapez « aide »
./target/release/my-erp tui     # TUI Ratatui : Tab modules, ↑↓ naviguer, f = formulaires (client, facture, devis, paiement, article, vente, société, FEC), n = nouveau, v valider, p payer, q quitter
./target/release/my-erp /clients nouveau --nom "Atelier Dupont" --type pro
./target/release/my-erp /factures   # commande unique, scriptable
bash scripts/scenario-demo.sh    # démonstration complète commentée
```

Données persistées dans `./donnees/etat.tsv` (dossier modifiable via
`ERP_DONNEES_DIR`, sauvegarde = copie du dossier). PostgreSQL : prévu comme
autre implémentation du trait `Store` (voir `decisions/ADR-002-persistance-v1.md`).
