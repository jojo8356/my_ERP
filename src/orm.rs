//! Couche ORM (ADR-006) : Rustorm sans base de données.
//!
//! Trois usages concrets, tous vérifiables hors ligne :
//! 1. **Modèle unique** — `rustorm/schema.rsm` est la seule description des
//!    tables ; il est validé et compilé en client typé à chaque build
//!    (`build.rs`), donc toute dérive du modèle casse la compilation.
//! 2. **Stockage local piloté par le modèle** — `fichier.rs` écrit un fichier
//!    tabulé par table dont les colonnes sont exactement `columns()` du modèle
//!    généré : aucune liste de colonnes n'est recopiée à la main.
//! 3. **Migration PostgreSQL prête** — `ddl_postgres()` produit le
//!    `CREATE TABLE` complet (via `rustorm::migrations::initial_sql`) sans
//!    jamais se connecter : le passage à PostgreSQL au jalon J5 consiste à
//!    exécuter ce SQL et à brancher un `Store` distant.
//!
//! `rustorm::checksum::sha256` sert aussi de primitive de hachage pour le
//! journal d'audit infalsifiable et les empreintes de sauvegarde (SEC-05).

/// Client typé généré à la compilation : `generated::client::Record`, etc.
#[allow(dead_code, clippy::all)]
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/rustorm_generated.rs"));
}

/// Empreinte courte du schéma compilé (16 caractères hexadécimaux).
pub const EMPREINTE_SCHEMA: &str = env!("RUSTORM_SCHEMA_CHECKSUM");

/// Source `.rsm` embarquée : `/admin schema` l'affiche telle quelle.
pub const SCHEMA_RSM: &str = include_str!("../rustorm/schema.rsm");

/// Modèle physiques connus de l'ERP (nom de table, colonnes générées).
///
/// C'est le seul endroit du code où les tables sont énumérées : les colonnes
/// viennent du client généré, jamais d'une liste écrite à la main.
pub fn tables() -> Vec<(&'static str, &'static [&'static str])> {
    vec![
        ("societes", generated::societe::columns()),
        ("utilisateurs", generated::utilisateur::columns()),
        ("clients", generated::client::columns()),
        ("notes", generated::note::columns()),
        ("documents", generated::document::columns()),
        ("lignes", generated::ligne::columns()),
        ("paiements", generated::paiement::columns()),
        ("ecritures", generated::ecriture::columns()),
        ("articles", generated::article::columns()),
        ("ventes", generated::vente::columns()),
        ("taux_change", generated::taux_change::columns()),
        ("operations_bancaires", generated::operation_bancaire::columns()),
        ("audit_log", generated::entree_audit::columns()),
        ("sequences", generated::sequence::columns()),
        ("sauvegardes", generated::sauvegarde::columns()),
        ("recurrences", generated::recurrence::columns()),
        ("meta", generated::meta::columns()),
    ]
}

/// Colonnes d'une table (`None` si la table n'existe pas dans le schéma).
pub fn colonnes(table: &str) -> Option<&'static [&'static str]> {
    tables()
        .into_iter()
        .find(|(nom, _)| *nom == table)
        .map(|(_, colonnes)| colonnes)
}

/// Schéma analysé (validation à l'exécution : utile pour `/admin schema`).
pub fn schema() -> Result<rustorm::schema::Schema, String> {
    rustorm::schema::parse(SCHEMA_RSM).map_err(|e| format!("schéma .rsm invalide : {e}"))
}

/// Migration initiale PostgreSQL (DDL complet, clés primaires et index).
///
/// Générée hors ligne : aucune connexion n'est ouverte. C'est le SQL que le
/// jalon J5 exécutera sur le serveur interne (CDC §5.1, ENF-FIA-01).
pub fn ddl_postgres() -> Result<String, String> {
    let schema = schema()?;
    rustorm::migrations::initial_sql(&schema)
        .map_err(|e| format!("génération de la migration impossible : {e:?}"))
}

/// Type PostgreSQL d'une colonne, tel que le produisent les migrations Rustorm.
fn type_postgres(ty: &rustorm::schema::Scalar) -> &'static str {
    use rustorm::schema::Scalar;
    match ty {
        Scalar::Int => "INTEGER",
        Scalar::BigInt => "BIGINT",
        Scalar::Float => "DOUBLE PRECISION",
        Scalar::Boolean => "BOOLEAN",
        Scalar::String | Scalar::Enum(_) => "TEXT",
        Scalar::DateTime => "TIMESTAMPTZ",
        Scalar::Bytes => "BYTEA",
        Scalar::Json => "JSONB",
        Scalar::Decimal { .. } => "NUMERIC",
    }
}

/// Migration **additive** : ajoute au serveur les colonnes apparues dans le
/// modèle depuis la création de la base.
///
/// `initial_sql` ne crée que les tables manquantes ; sans cette passe, une base
/// existante resterait sur l'ancien modèle et la lecture échouerait. Chaque
/// instruction est idempotente (`IF NOT EXISTS`), donc rejouable à chaque
/// démarrage, et n'ajoute `NOT NULL` que si une valeur par défaut existe — une
/// colonne ajoutée ne peut ainsi jamais invalider des lignes déjà présentes.
pub fn alter_postgres() -> Result<String, String> {
    let schema = schema()?;
    let mut sql = String::from("-- Colonnes ajoutées au modèle depuis la création de la base.\n");
    for model in &schema.models {
        for field in &model.fields {
            let mut instruction = format!(
                "ALTER TABLE \"{}\" ADD COLUMN IF NOT EXISTS \"{}\" {}",
                model.table.replace('"', ""),
                field.column.replace('"', ""),
                type_postgres(&field.ty)
            );
            let defaut = match &field.default {
                Some(rustorm::schema::DefaultValue::String(v)) => {
                    Some(format!("'{}'", v.replace('\'', "''")))
                }
                Some(rustorm::schema::DefaultValue::Number(v)) => Some(v.clone()),
                Some(rustorm::schema::DefaultValue::Boolean(v)) => Some(v.to_string()),
                Some(rustorm::schema::DefaultValue::Now) => Some("now()".to_string()),
                _ => None,
            };
            if let Some(defaut) = &defaut {
                instruction.push_str(&format!(" DEFAULT {defaut}"));
                if !field.nullable && !field.id {
                    instruction.push_str(" NOT NULL");
                }
            }
            instruction.push_str(";\n");
            sql.push_str(&instruction);
        }
    }
    Ok(sql)
}

/// SHA-256 hexadécimal (primitive partagée : audit, sauvegardes, empreintes).
pub fn sha256(donnees: &[u8]) -> String {
    rustorm::checksum::sha256(donnees)
}

/// Empreinte courte et lisible d'un texte.
pub fn empreinte(texte: &str) -> String {
    let complet = sha256(texte.as_bytes());
    complet[..16].to_string()
}

/// Description texte du modèle (utilisée par `/admin schema` et les tests).
pub fn resume() -> String {
    let mut sortie = format!(
        "Rustorm {} — schéma {} (empreinte {})\n",
        env!("CARGO_PKG_VERSION"),
        "rustorm/schema.rsm",
        EMPREINTE_SCHEMA
    );
    for (table, colonnes) in tables() {
        sortie.push_str(&format!(
            "  {table:<22} {:>2} colonnes  ({})\n",
            colonnes.len(),
            colonnes.first().copied().unwrap_or("-")
        ));
    }
    sortie
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_schema_est_valide_et_complet() {
        let schema = schema().expect("le .rsm doit être valide");
        assert!(schema.models.len() >= 15, "modèles : {}", schema.models.len());
        // Le CDC §8 exige ces entités : aucune ne doit disparaître du modèle.
        for attendu in [
            "societes",
            "utilisateurs",
            "clients",
            "documents",
            "lignes",
            "paiements",
            "ecritures",
            "articles",
            "ventes",
            "taux_change",
            "audit_log",
            "sequences",
            "meta",
        ] {
            assert!(
                schema.models.iter().any(|m| m.table == attendu),
                "table manquante dans le schéma : {attendu}"
            );
        }
    }

    #[test]
    fn le_client_genere_expose_les_colonnes() {
        let clients = colonnes("clients").expect("table clients");
        assert!(clients.contains(&"nom"));
        assert!(clients.contains(&"etiquettes"));
        assert!(colonnes("table_inexistante").is_none());
    }

    #[test]
    fn la_migration_postgres_est_generable_sans_base() {
        let ddl = ddl_postgres().expect("DDL");
        assert!(ddl.contains("CREATE TABLE \"clients\""));
        assert!(ddl.contains("CREATE TABLE \"ecritures\""));
        assert!(ddl.contains("PRIMARY KEY"));
        assert!(ddl.contains("CREATE INDEX"));
    }

    #[test]
    fn la_migration_additive_est_rejouable() {
        let sql = alter_postgres().expect("ALTER");
        // Chaque table du modèle et chaque colonne y passent, sans jamais
        // supprimer ni renommer quoi que ce soit.
        for (table, colonnes) in tables() {
            assert!(sql.contains(&format!("ALTER TABLE \"{table}\"")), "{table} absent");
            for colonne in colonnes {
                assert!(
                    sql.contains(&format!("ADD COLUMN IF NOT EXISTS \"{colonne}\"")),
                    "{table}.{colonne} absent"
                );
            }
        }
        assert!(sql.contains("IF NOT EXISTS"));
        assert!(!sql.to_uppercase().contains("DROP"), "une migration ne supprime rien");
        // Une colonne NOT NULL n'est ajoutée qu'avec une valeur par défaut.
        for ligne in sql.lines().filter(|l| l.contains("NOT NULL")) {
            assert!(ligne.contains("DEFAULT"), "sans défaut : {ligne}");
        }
    }

    #[test]
    fn empreintes_stables() {
        assert_eq!(sha256(b"abc").len(), 64);
        assert_eq!(empreinte("abc"), empreinte("abc"));
        assert_ne!(empreinte("abc"), empreinte("abd"));
    }
}
