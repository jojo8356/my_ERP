# Conformité au CDC v1.1 — état réel, exigence par exigence

Ce fichier est la **traçabilité** demandée au §9 du CDC : pour chaque exigence
identifiée, où elle est implémentée, comment elle est vérifiée, et ce qui reste
ouvert. Il décrit l'état du code, pas une intention.

Statuts : **✔ tenu** (implémenté et testé) · **◑ partiel** (implémenté avec une
limite explicite) · **✘ non couvert** (assumé en v1.2, justification donnée).

Vérification de référence : `cargo test` (111 tests) et
`bash scripts/scenario-demo.sh` (recette de bout en bout, process séparés),
tous deux exécutés par la CI (`.github/workflows/ci.yml`).

## Socle

| Exigence | Statut | Où | Vérifié par |
|---|---|---|---|
| EF-SOC-01 Multi-sociétés, données cloisonnées, devise par société | ✔ | `store.rs` (`basculer_societe`, filtres `societe_id`), `cli.rs` (`/societes`, `/societes/<nom>`, `--societe`) | `store::tests::multi_societes_cloisonnees`, `cli::tests::bascule_de_societe_par_le_chemin`, scénario §14 |
| EF-SOC-02 Multi-devises, taux datés, totaux en devise de référence | ✔ | `domain.rs` (`convertir`, `parse_taux`, `Document::totaux_reference`), `store.rs` (`enregistrer_taux`, `taux`), `/devises` | `domain::tests::conversions_devises`, `store::tests::devises_et_conversion`, scénario §13 |
| EF-SOC-03 Recherche globale | ✔ | `store::recherche_globale`, `/recherche`, TUI (`s`) | `cli::tests::recherche_et_listes_json`, scénario §11 |

## Module B — Facturation et devis

| Exigence | Statut | Où | Vérifié par |
|---|---|---|---|
| EF-FAC-01 Devis : statuts brouillon/envoyé/accepté/refusé/expiré, transformation en 1 commande | ✔ | `domain::StatutDocument`, `store::changer_statut`, `transformer_devis`, `/devis <n°> envoyer\|accepter\|refuser\|expirer\|transformer` | `cli::tests` (parcours devis), scénario §10 |
| EF-FAC-02 Numérotation continue par société et exercice, immuabilité | ✔ | `store::prochaine_sequence` (clé société/type/année), refus des doublons, `message` d'erreur proposant l'avoir | `store::tests::numerotation_sans_trou_et_immuabilite`, `multi_societes_cloisonnees` |
| EF-FAC-03 Lignes (désignation, quantité, PU HT, remise, TVA par ligne, totaux) | ✔ | `domain::Ligne`/`totaux`/`tva_par_taux`, `cli::parser_ligne` | `domain::tests` (totaux, remise, ventilation), `cli::tests::parse_ligne_ok` |
| EF-FAC-04 Échéance, conditions, pénalités, indemnité 40 € paramétrables | ◑ | `Societe.delai_paiement_jours`/`penalites_taux_bp`/`indemnite_forfaitaire`, `Document` les recopie, `domain::penalites_retard` | `domain::tests::paiements_partiels_et_penalites` · limite : les **acomptes avant facture** ne sont pas modélisés (seuls les paiements après émission le sont) |
| EF-FAC-05 PDF conforme + envoi e-mail en 1 commande | ✔ | `pdf::facture`, `messagerie::courriel_document`, `/factures <n°> pdf\|envoyer` | `pdf::tests`, `messagerie::tests`, scénario §3 et §5 · limite : envoi = dépôt dans `donnees/courriels/*.eml` (pas de SMTP, ADR-008) |
| EF-FAC-06 Suivi des paiements, impayés, retards, relances | ✔ | `store::documents_impayes`/`documents_en_retard`/`ajouter_paiement`, `messagerie::courriel_relance` (3 niveaux), `/factures <n°> payer\|relance`, `--impayees`, `--retard` | `store::tests`, `messagerie::tests::relances_graduees`, scénario §6 |
| EF-FAC-07 Export e-facture structuré (UBL/EN 16931) | ✔ | `efacture::ubl`, `/factures <n°> ubl` | `efacture::tests` (4), scénario §3 · limite : transmission via plateforme agréée = jalon J6 (dépend du prestataire) |
| EF-FAC-08 Factures récurrentes | ✔ | table `recurrences`, `store::creer_recurrence`/`generer_recurrences`, `/factures recurrentes …` | `store::tests` (récurrences), `cli::tests` |

## Module A — CRM

| Exigence | Statut | Où | Vérifié par |
|---|---|---|---|
| EF-CRM-01 Fiche client complète (SIREN, TVA intracom, type, adresses, notes) | ✔ | `domain::Client`, `fichier::encoder_client`, `/clients nouveau\|<nom> modifier` | `cli::tests`, `store::tests` |
| EF-CRM-02 Historique unifié (devis, factures, paiements, ventes) | ✔ | `store::historique_client`, `/clients <nom>` et `/clients <nom> factures` | `store::tests::historique_client_unifie` |
| EF-CRM-03 Étiquettes, recherche, export CSV | ✔ | `etiqueter`/`retirer_etiquette`, `--tag`, `/clients exporter` | `cli::tests` (tags, export), scénario |

## Module C — Comptabilité / Finance

| Exigence | Statut | Où | Vérifié par |
|---|---|---|---|
| EF-CPT-01 Écritures automatiques (plan comptable FR, journaux VE/BQ/AC) | ✔ | `compta::ecritures_depuis` (411000/401000/512000/706000/707000/445710/445660/607000/370000/627000), `store::regenerer_ecritures` | `compta::tests` (6), `store::tests` |
| EF-CPT-02 TVA collectée/déductible par taux et période | ✔ | `compta::recap_tva_par_taux`, `recap_tva_ecritures`, `/compta tva [période]` | `compta::tests::recap_tva_par_taux_et_periode`, scénario §8 |
| EF-CPT-03 Export FEC conforme + journaux | ✔ | `compta::fec`/`journal`/`journaux`, `/compta fec [fichier]` | `compta::tests` (18 colonnes, montants `1234,56`), scénario §8 |
| EF-CPT-04 Rapprochement bancaire (import + lettrage semi-auto) | ◑ | `store::importer_releve`/`lettrer`, `/compta rapprocher\|lettrer` | `store::tests`, scénario §9 · limite : **CSV uniquement** (pas d'OFX) |
| EF-CPT-05 Tableau de bord finance | ✔ | `store::ca_mensuel`/`encours`/`tresorerie`, `/compta tableau`, `/` (tableau de bord) | `cli::tests`, scénario §8 |

## Module D — Vinted

| Exigence | Statut | Où | Vérifié par |
|---|---|---|---|
| EF-VIN-01 Article (photos, marque, taille, état, composition, prix/date d'achat, source, statut) | ✔ | `domain::Article`, `/vinted nouveau`, `/vinted statut <n°> en_stock\|en_ligne\|vendu` | `cli::tests::statut_d_article_vinted`, scénario §7 |
| EF-VIN-02 Vente, frais, marge nette, régime par article | ✔ | `domain::marge_nette`, `Vente.regime`, `/vinted vendre` | `domain::tests`, `cli::tests::vinted_par_le_cli` |
| EF-VIN-03 Import CSV des ventes | ✔ | `cli::importer_ventes`, `/vinted importer` | `cli::tests`, scénario |
| EF-VIN-04 Statistiques + rotation + alerte DAC7 | ✔ | `store::stats_vinted` (marges par marque/taille, `taux_ecoulement`, `delai_moyen_jours`), `domain::alerte_dac7`, `/vinted stats` | `store::tests::stats_vinted`, `domain::tests` (DAC7), scénario §7 |
| EF-VIN-05 Fiche produit exportable | ✔ | `Article::annonce`, `/vinted fiche <n°>` | `cli::tests`, scénario |

## Intégrations

| Exigence | Statut | Où | Commentaire |
|---|---|---|---|
| INT-01 E-mail (devis/factures, relances, codes de connexion) | ◑ | `messagerie.rs` (RFC 5322 + MIME, PDF base64) | Message complet écrit dans `donnees/courriels/` ; **transmission SMTP = jalon J6** (pas de TLS dans l'environnement de V1). Voir ADR-008 |
| INT-02 E-facture : émission | ✔ | `efacture::ubl` | export UBL/EN 16931 |
| INT-02 E-facture : **réception** | ✘ | — | non implémentée : dépend de la plateforme agréée (§12) et suppose un modèle fournisseur/achat. Reportée en v1.2, tracée ici plutôt que masquée |
| INT-03 CSV/Excel (imports clients, articles, ventes ; exports partout) | ✔ | `csv.rs`, `/clients importer\|exporter`, `/vinted importer\|exporter`, `--csv` sur toutes les listes, FEC | `csv::tests`, scénario |
| INT-04 Stripe (liens de paiement) | ✘ | — | (*S*) lié à un compte Stripe et à un coffre de jetons : aucun secret n'est stocké en V1 |
| INT-05 Banque (relevés pour rapprochement) | ◑ | `/compta rapprocher <releve.csv>` | CSV seulement (voir EF-CPT-04) |
| INT-06 Signature électronique · INT-07 SMS | ✘ | — | classés **C** (v1.x) dans le CDC : hors périmètre v1 |

## Exigences non fonctionnelles

| Exigence | Statut | Où / comment |
|---|---|---|
| ENF-PERF-01 Commande courante < 500 ms, démarrage < 300 ms | ◑ | Aucune requête réseau, aucune base : lecture d'une poignée de fichiers TSV (< 10 000 lignes) par commande. Non mesuré automatiquement (pas d'outil de benchmark hors-ligne) ; volume cible tenu par construction |
| ENF-SIMPL-01 Binaire unique, installation simple, code métier couvert | ◑ | `cargo build --release` → un seul binaire ; les **règles de calcul sont couvertes à 100 %** (montants, TVA, remises, pénalités, amortissement des avoirs). Couverture globale **non instrumentée** (pas de `tarpaulin` dans l'environnement) : 111 tests |
| ENF-ERG-01 Aide contextuelle, messages français actionnables | ✔ | `aide_contexte` (`/aide <chemin>`, `?`), tous les `Err` proposent la commande à lancer ; tests qui vérifient le texte des messages |
| ENF-FIA-01 Pas d'écriture à moitié enregistrée, reprise après coupure | ◑ | Écriture atomique fichier temporaire + renommage (`fichier::ecrire_table`), sauvegardes datées avec empreinte. Limite : pas de transaction multi-tables (une coupure entre deux tables se répare par restauration) |
| ENF-SCAL-01 ×2 utilisateurs, ×10 lignes, module ajoutable | ◑ | Comptes/rôles prêts ; 10 000 lignes/module tiennent en mémoire. Limite : stockage **mono-processus** (ADR-006) — le passage à PostgreSQL est la réponse prévue |
| ENF-COMPAT-01 Linux/macOS/Windows, exports ouverts | ✔ | Rust std + `crossterm`/`ratatui` (multiplateforme) ; exports CSV, PDF, UBL, FEC |
| ENF-PORT-01 Données exportables, aucun verrouillage | ◑ | TSV lisibles, CSV/PDF/UBL/FEC, import de l'ancien `etat.tsv` v1. Le « dump PostgreSQL » du CDC n'a pas d'objet tant qu'il n'y a pas de base (ADR-002/006) |
| ENF-COUT-01 100 % open source, 0 licence | ✔ | Dépendances : Chromat et Rustorm (auteur du projet), Ratatui/Crossterm (MIT), aucune licence payante |

## Sécurité et RGPD

| Exigence | Statut | Où | Vérifié par |
|---|---|---|---|
| SEC-01 Comptes nominatifs | ✔ | table `utilisateurs`, `/utilisateurs`, `/auth` | `store::tests::utilisateurs_et_roles` |
| SEC-02 Double authentification (admin) | ✔ | `securite::totp`/`verifier_totp`, `/utilisateurs 2fa`, connexion `--totp` | `securite::tests` (RFC 6238), scénario §12 (connexion par code de secours) |
| SEC-03 Mots de passe hachés | ✔ | PBKDF2-HMAC-SHA256 120 000 itérations + sel, `egalite_constante` | `securite::tests::hachage_de_mot_de_passe_aller_retour` |
| SEC-04 Rôles et permissions | ✔ | `Role`, `Store::exiger_admin` (validation, restauration, comptes) | `store::tests`, `cli::tests` |
| SEC-05 Journal d'audit infalsifiable | ✔ | `store::journaliser` (chaîne SHA-256), `verifier_chaine_audit`, `/admin audit [--verifier]` | `store::tests::audit_chaine_et_alteration_detectee`, scénario §12 |
| SEC-06 Aucune donnée bancaire complète | ✔ | Le relevé importé ne contient que date/libellé/montant/référence ; aucun identifiant de carte ni jeton | `store::tests` (import de relevé) |
| RGPD-01 Export des données personnelles | ✔ | `store::exporter_client`, `/clients <nom> rgpd exporter` | `store::tests` |
| RGPD-02 Effacement / anonymisation | ✔ | `store::effacer_client_rgpd` (anonymise, conserve 10 ans les pièces comptables), `/clients <nom> rgpd effacer` | `store::tests` |
| RGPD-03 Journalisation des accès | ✔ | Chaque export/effacement est journalisé (`rgpd.export`, `rgpd.effacement`) | `store::tests` |

## Ce qui reste ouvert (assumé, à trancher au jalon J6)

1. **INT-02 réception** d'e-factures fournisseurs : nécessite la plateforme
   agréée et un modèle « achat/fournisseur » (le schéma ne le porte pas encore).
2. **SMTP réel** (INT-01) : `messagerie::enveloppe()` produit le message
   complet ; il manque uniquement le transport authentifié.
3. **PostgreSQL** : migration prête (`/admin schema --sql`), stockage
   mono-processus en attendant (ADR-006).
4. **Acomptes avant facture** (EF-FAC-04) : seuls les paiements après émission
   sont gérés.
5. **OFX** (EF-CPT-04) : CSV seulement.
6. **Mesure de couverture et de performance** : non instrumentée hors-ligne.
