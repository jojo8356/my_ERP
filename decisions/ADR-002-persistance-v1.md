# ADR-002 — Persistance de la V1 : fichier local, PostgreSQL différé

Date : 30/10/2026 · Statut : acté pendant le développement de la V1 (à valider par l'utilisateur)

## Contexte
Le CDC (§5.1) prévoit PostgreSQL. Or l'environnement de développement/CI utilisé
pour coder la V1 bloque les téléchargements externes (crates.io, images Docker) :
le pilote PostgreSQL Rust (`postgres`) y est intéléchargeable, ce qui rendait la
compilation impossible à vérifier.

## Décision
La V1 est écrite en **Rust 100 % bibliothèque standard (zéro dépendance)**.
La persistance est un **stockage fichier local** (`Fichier`, fichiers tabulés
dans un dossier de données, écriture atomique), derrière le trait `Store`
existant — exactement comme `Memoire` (tests).

## Conséquences
- Compilation et tests reproductibles partout, hors-ligne, sans aucun téléchargement.
- Données persistées entre les exécutions, mono-utilisateur (conforme au périmètre v1 : 1 utilisateur).
- Le passage à PostgreSQL = une nouvelle implémentation du trait `Store`, sans toucher
  ni au métier (`domain.rs`) ni au CLI (`cli.rs`). Prévu au jalon de durcissement (J5)
  quand l'environnement d'exécution réel (serveur interne) sera utilisé.
- Sauvegarde = copie du dossier de données (compatible avec la sauvegarde quotidienne du CDC).

## Alternative écartée
Attendre un environnement avec accès crates.io : bloquait toute la V1.
