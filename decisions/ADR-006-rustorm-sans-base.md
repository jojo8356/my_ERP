# ADR-006 — Rustorm comme modèle unique, **sans base de données**

Date : 02/11/2026 · Statut : acté (demande explicite de l'utilisateur : « implémente
tout ce qui manquait sans base de données »)

## Contexte

Le CDC (§5.1) prévoit PostgreSQL, mais la V1 doit tourner **sans serveur de base
de données** : poste unique, poste hors-ligne, CI sans service externe. En même
temps, un ERP ne peut pas se permettre deux descriptions divergentes du modèle
(une pour le code, une pour la base) : c'est la source classique des bugs de
colonnes et des migrations oubliées.

L'auteur du projet a par ailleurs publié **Rustorm**
(<https://github.com/jojo8356/rustorm>, tag `v0.1.0`) : un ORM « à la Prisma »
en Rust, dont le schéma `*.rsm` est validé, transformé en client typé et en
migration SQL — le tout **sans exiger de connexion** pour les parties schéma.

## Décision

`rustorm/schema.rsm` est **la seule description du modèle** (15 tables). Le
schéma n'est pas un document : il est consommé à la compilation.

1. **`build.rs`** lit `rustorm/schema.rsm`, le valide
   (`rustorm::schema::parse`), génère le client typé
   (`rustorm::schema::generate` → `$OUT_DIR/rustorm_generated.rs`) et fixe
   l'empreinte SHA-256 du schéma dans une variable d'environnement de
   compilation (`RUSTORM_SCHEMA_CHECKSUM`).
2. **`src/orm.rs`** inclut ce client (`orm::generated::<modele>::Record`,
   `::columns()`), expose `tables()`, `colonnes()`, `schema()`, `resume()` et
   `ddl_postgres()`. Une faute dans le `.rsm` **casse le build** : aucune dérive
   silencieuse entre le modèle et le code.
3. **`src/fichier.rs`** est le moteur de stockage local : une table du schéma =
   un fichier `donnees/tables/<table>.tsv`, dont **l'en-tête est exactement la
   liste `columns()` du modèle généré**. Les lignes sont écrites par nom de
   colonne (`Ecrit::t`), et `fin()` refuse une colonne inventée ou oubliée — le
   compilateur ne connaît pas les colonnes, mais le schéma les vérifie à
   l'exécution et les tests font l'aller-retour complet.
4. **La migration PostgreSQL est déjà écrite** :
   `orm::ddl_postgres()` renvoie le `CREATE TABLE` complet produit par
   `rustorm::migrations::initial_sql`. Passer à PostgreSQL (jalon J5) = exécuter
   ce SQL et brancher un magasin distant : ni le métier (`domain.rs`) ni le CLI
   (`cli.rs`) ne changent.
5. La feature `postgres` de Rustorm est **volontairement absente** de
   `Cargo.toml` (elle est présente en commentaire). Sans elle, aucun pool, aucun
   pilote, aucune connexion ne peut être instanciée : l'absence de base est
   garantie par le système de features, pas par une convention.

### Pourquoi pas de relations `@relation` dans le schéma ?

Rustorm valide les relations deux à deux (cible, inverse, actions
référentielles). Le schéma décrit ici les **colonnes de jointure**
(`client_id`, `document_id`, `societe_id`…) et l'intégrité référentielle est
tenue par le magasin (`Store::exiger_client`, refus des numéros dupliqués,
interdiction de supprimer une pièce comptable). Ce choix garde le `.rsm`
compréhensible sans base et évite de simuler en mémoire des contraintes que
seul PostgreSQL pourra réellement faire respecter ; les clés étrangères
reviennent avec la migration au J5.

## Conséquences

- **Une seule vérité** : ajouter une colonne = modifier `schema.rsm` ; le client
  typé, l'en-tête des TSV et le DDL changent ensemble, ou rien ne compile.
- **Zéro service** : `cargo test` et le scénario de recette tournent hors-ligne,
  sur un dossier de données, sans `DATABASE_URL`.
- **Coût assumé** : Rustorm est compilé deux fois (dépendance + dépendance de
  build) et tire des dépendances transitives inutilisées à l'exécution
  (`tokio`/`deadpool` du connecteur). Le binaire ne contient aucun serveur : ces
  crates ne sont jamais instanciées faute de feature.
- **Pas de SQL à la main** dans le code applicatif ; le seul SQL du dépôt est
  celui que Rustorm génère pour le jalon J5.
- **Limite connue** : le stockage TSV est mono-processus (verrou absent). V2 :
  soit un fichier verrouillé, soit PostgreSQL — la migration est prête.

## Alternatives écartées

- **PostgreSQL en v1** : viole la contrainte « sans base de données » et rend la
  CI dépendante d'un service.
- **Modèle écrit à la main** (`struct` + listes de colonnes) : exactement le
  doublon que cet ADR supprime, et le `.rsm` ne serait plus vérifié.
- **SQLite** : nouvelle dépendance native, toujours un fichier, mais sans le
  bénéfice du modèle unique déjà fourni par Rustorm.
