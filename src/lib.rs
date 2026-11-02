//! my_ERP — bibliothèque de l'ERP en ligne de commande (CDC-ERP-CLI.md v1.1).
//!
//! Organisation des modules, du plus pur au plus proche du terminal :
//!
//! - [`domain`]   : règles métier pures (montants en centimes, dates, TVA,
//!   seuils micro-entreprise, mentions obligatoires, totaux). Aucune E/S.
//! - [`securite`] : primitives sans dépendance (SHA-1/SHA-256, HMAC, PBKDF2,
//!   base32, TOTP, codes de secours) — SEC-* du cahier des charges.
//! - [`fichier`]  : moteur TSV aligné sur le schéma Rustorm (écriture atomique,
//!   lecture tolérante, `meta.tsv`), utilisé par le [`store`] local.
//! - [`orm`]      : client Rustorm généré à la compilation depuis
//!   `rustorm/schema.rsm` (empreinte, colonnes, DDL PostgreSQL).
//! - [`store`]    : magasin unique (mémoire ou dossier), multi-société,
//!   documents, paiements, comptabilité, audit chaîné, sauvegardes.
//! - [`compta`]   : plan comptable, journaux, FEC, balance, TVA.
//! - [`csv`]      : export CSV (RFC 4180) des listes.
//! - [`pdf`]      : génération PDF sans dépendance (factures conformes).
//! - [`messagerie`] : e-mails RFC 5322/MIME et boîte d'envoi locale.
//! - [`efacture`] : e-facture UBL 2.1 / EN 16931.
//! - [`theme`]    : couleurs Chromat (aucun code ANSI ailleurs).
//! - [`cli`]      : navigation par chemins, options, pagination, REPL.
//! - [`tui`]      : écran plein terminal (Ratatui), offre visuelle du CLI.

pub mod cli;
pub mod compta;
pub mod csv;
pub mod domain;
pub mod efacture;
pub mod fichier;
pub mod messagerie;
pub mod orm;
pub mod pdf;
pub mod securite;
pub mod store;
pub mod theme;
pub mod tui;

pub use cli::App;
pub use domain::{Cents, Document, Ligne, Societe};
pub use store::Store;
