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

Zéro GUI, **aucune base de données**. Couleurs : [Chromat](https://github.com/jojo8356/chromat)
(bibliothèque de l’auteur, ADR-004) pilote le CLI via `src/theme.rs` ; la TUI
utilise **Ratatui + Crossterm** (ADR-005) avec la même palette et des
**formulaires** pour chaque création/action. Le modèle de données est un schéma
[**Rustorm**](https://github.com/jojo8356/rustorm) (`rustorm/schema.rsm`, 15 tables)
validé **et compilé à chaque build** : le client typé, les colonnes des fichiers
de données et la future migration PostgreSQL viennent tous du même fichier
(ADR-006). Aucune connexion n’est ouverte : le stockage local est un moteur
tabulaire piloté par le schéma.

Navigation par **chemins type URL** et raccourcis, comme exigé dans le
[CDC](CDC-ERP-CLI.md). Périmètre couvert : CRM, devis/factures/avoirs, PDF
conforme + e-facture UBL/EN 16931, envoi e-mail (RFC 5322), relances graduées,
comptabilité (écritures, TVA, FEC, balance, rapprochement bancaire), seuils
micro-entreprise, achat-revente Vinted (marges, rotation, alerte DAC7),
multi-sociétés, multi-devises, utilisateurs/rôles/2FA et journal d’audit chaîné.
Le détail exigence par exigence est dans **[CONFORMITE-CDC.md](CONFORMITE-CDC.md)**.

```bash
cargo build --release            # binaire unique : target/release/my-erp
./target/release/my-erp          # mode interactif (erp:/chemin>), tapez « aide »
./target/release/my-erp tui      # TUI : ←→ onglets, ↑↓ défiler, n nouveau, p paiement, s recherche, ? aide, q quitter
./target/release/my-erp /clients nouveau --nom "Atelier Dupont" --type pro
./target/release/my-erp /factures --impayees --retard      # commande unique, scriptable
./target/release/my-erp /factures FA-2026-0001 pdf facture.pdf
./target/release/my-erp /compta fec > fec.csv              # FEC DGFiP
./target/release/my-erp /admin schema --sql                # migration PostgreSQL prête
bash scripts/scenario-demo.sh    # recette complète commentée (15 étapes)
```

Sorties machine : `--json` et `--csv` sur toutes les listes, pagination
`--page`/`--lignes`, `NO_COLOR` respecté, code de sortie 0/1.

### Où sont les données

```
donnees/
  meta.tsv               société courante, session, compteur d'identifiants, version
  tables/*.tsv           une table du schéma = un fichier (colonnes = celles du .rsm)
  courriels/*.eml        messages générés (facture, devis, relance, code de connexion)
  sauvegardes/           copies datées (empreinte SHA-256 vérifiable)
```

Dossier modifiable via `ERP_DONNEES_DIR` ou `--donnees <dossier>` ; `--memoire`
= démonstration sans écriture disque. L’ancien `etat.tsv` de la v1 est importé
automatiquement au premier démarrage (ENF-PORT-01). Sauvegarde = `/admin
sauvegarde` (ou copie du dossier) ; restauration = `/admin restaurer <nom>`.

PostgreSQL n’est **pas** requis : la migration complète
(`/admin schema --sql`, produite par Rustorm) est prête pour le jour où un
serveur existera (jalons J5/J6, `decisions/ADR-002` et `ADR-006`).

### Documentation des décisions

`decisions/` : ADR-002 (persistance locale), ADR-003 (TUI), ADR-004 (Chromat),
ADR-005 (Ratatui/formulaires), **ADR-006** (Rustorm sans base), **ADR-007**
(sécurité locale : PBKDF2, TOTP, audit chaîné), **ADR-008** (PDF/e-mail/CSV/
e-facture sans dépendance). Traçabilité du CDC : `CONFORMITE-CDC.md`.
