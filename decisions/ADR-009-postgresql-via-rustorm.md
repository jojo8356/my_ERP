# ADR-009 — PostgreSQL **par l'ORM**, en option de compilation

Date : 02/11/2026 · Statut : acté (demande explicite : « utilise l'ORM que je t'ai
dit d'utiliser pour les bases de données »)

## Contexte

ADR-006 avait fait de `rustorm/schema.rsm` le modèle unique, mais le connecteur
PostgreSQL de Rustorm n'était pas activé : l'ERP ne savait pas écrire dans une
vraie base. Or le CDC (§5.1, §8) prévoit PostgreSQL comme base cible : les clés
étrangères strictes, l'unicité des numéros de facture et les transactions
multi-tables (ENF-FIA-01) ne peuvent pas être garanties par des fichiers TSV.

La contrainte inverse reste vraie : l'ERP doit continuer à fonctionner **sans
serveur** (poste unique, hors-ligne, CI sans service). Les deux modes doivent
donc coexister sans dupliquer ni le métier, ni le CLI, ni la TUI.

## Décision

Le support PostgreSQL est ajouté **derrière la feature `postgres`**, désactivée
par défaut (`cargo build --features postgres` l'active). Tout passe par l'ORM,
aucune requête de données n'est écrite à la main dans le code applicatif :

| Besoin | Utilisé | Fichier |
|---|---|---|
| noms de tables, colonnes, types | client généré (`columns()`) + `rustorm::schema` | `src/orm.rs` |
| `CREATE TABLE`, index, clés primaires | `rustorm::migrations::initial_sql` | `orm::ddl_postgres()` |
| connexion, paramètres liés, erreurs structurées | `rustorm::postgres::Connection` | `src/bdd.rs` |
| transactions (écriture multi-tables atomique) | `Connection::begin` → `Transaction::execute` → `commit` / `rollback` | `src/bdd.rs` |
| lecture | constructeur `rustorm::query::Select` + `Row` (typé par colonne) | `src/bdd.rs` |
| écriture | `rustorm::query::CompiledQuery` + `Value` typés d'après le schéma | `src/bdd.rs` |

Points de conception :

1. **Un seul encodage, deux supports.** `Store::tables_encodees()` produit les
   lignes du magasin dans l'ordre des colonnes du modèle ; `fichier::ecrire_table`
   les écrit en TSV, `bdd::Bdd::enregistrer_tout` les rejoue en SQL paramétré.
   Ajouter une colonne au `.rsm` la fait apparaître, ou casser la compilation,
   mais ne peut pas être persistée dans un seul des deux modes.
2. **Décodage partagé.** `fichier::decoder_*` reçoit une `LigneTable`, qu'elle
   vienne d'un TSV ou de PostgreSQL (`LigneTable::depuis_valeurs`) : les règles
   de conversion (centimes, points de base, booléens `t`/`f`) sont écrites une
   fois.
3. **Le schéma crée la base.** `Store::ouvrir_bdd` applique la migration si
   `clients` n'existe pas encore : la base est initialisée par l'ORM, jamais par
   un script SQL écrit à la main. La migration reste rejouable (`/admin bdd`).
4. **Écriture transactionnelle.** Chaque `persister()` remplace l'ensemble des
   tables dans **une transaction** : une panne au milieu ne laisse jamais la
   comptabilité à moitié enregistrée (ENF-FIA-01), ce que le mode fichier ne
   peut pas promettre entre deux tables.
5. **Même application.** `--bdd <url>` ou `ERP_DATABASE_URL` (ou l'option
   `/admin bdd`) changent uniquement le support : toutes les commandes, le CLI,
   la TUI, les exports PDF/UBL/FEC et le journal d'audit sont identiques.
6. **Sécurité de connexion.** Une base locale (`localhost`/`127.0.0.1`) est
   jointe sans TLS pour le développement et la CI ; toute autre cible passe par
   `Connection::connect`, qui impose `sslmode=verify-full` (SEC-07). Les URL
   affichées sont masquées (`postgres://erp:***@hote/base`).
7. **Sauvegarde.** En mode base, `/admin sauvegarde` et `/admin restaurer`
   refusent explicitement : la sauvegarde se fait par `pg_dump`/`pg_restore`
   (copie à chaud gérée par le serveur), avec un message qui explique la
   commande à utiliser plutôt qu'un échec silencieux.

## Conséquences

- **Le mode par défaut reste sans base** : aucun pilote n'est lié, aucun
  `libpq` requis, `cargo test` et la recette tournent hors-ligne. La CI couvre
  les deux : un job sans base (`test-all.sh`) et un job avec un service
  PostgreSQL 16 (`test-all.sh --features postgres --bdd …`).
- **Le connecteur Rustorm passe par `libpq`** (`#[link(name = "pq")]`) : la
  compilation avec la feature demande `libpq-dev` (Debian/Ubuntu) ou `libpq`
  (macOS/Homebrew). C'est le prix du connecteur officiel de l'ORM ; aucun
  pilote « pur Rust » n'est réimplémenté ici.
- **Les clés étrangères strictes du CDC §8** ne sont pas encore déclarées dans
  le `.rsm` (ADR-006, décision conservée) : l'intégrité est vérifiée par le
  magasin avant écriture. Les ajouter relève d'une migration ultérieure, à
  faire dans le schéma pour que les deux modes en héritent.
- **Tests d'intégration** : `src/bdd.rs` contient trois tests marqués
  `#[ignore]`, exécutés uniquement quand `ERP_DATABASE_URL` pointe sur une base
  jetable (`cargo test --features postgres -- --ignored`, ce que fait
  `test-all.sh --bdd`). Ils vérifient la migration complète, un parcours
  complet relu après fermeture du processus, et l'atomicité des écritures.
- **PostgreSQL n'est plus un jalon « plus tard »** : le mode existe, il est
  testé, et `--features postgres` suffit à basculer.

## Alternatives écartées

- **Toujours activer `postgres`** : imposerait `libpq` à tous les postes et
  contredirait la contrainte « sans base de données » du mode par défaut.
- **Écrire du SQL à la main** pour les insertions groupées : double le modèle,
  contourne les points 1 et 2, et perd la vérification `columns()`.
- **`sqlx` (driver pur Rust de Rustorm)** : disponible en feature, mais tire un
  runtime asynchrone complet pour des écritures séquentielles de CLI. Le
  connecteur `postgres` (libpq) suffit et reste synchrone.
