# ADR-003 — Interface TUI (terminal plein écran), toujours sans GUI

Date : 30/10/2026 · Demandé par l'utilisateur (« TUI ») après la V1 CLI.

## Décision
Ajout d'un mode `my-erp tui` : interface plein écran dans le terminal
(modules Tableau de bord / Clients / Factures / Vinted / Comptabilité),
navigation clavier (Tab, flèches/j-k, v = valider, p = payer, ? = aide, q = quitter).

## Technique
Rust 100 % bibliothèque standard (cohérent avec ADR-002) : séquences ANSI pour
l'écran alternatif et la sélection inversée, `stty` pour le mode raw.
Aucune dépendance externe (ratatui écarté : crates.io inaccessible dans
l'environnement de développement/CI). Le rendu est une fonction pure testée
unitairement ; les lectures/écritures passent par le trait `Store`, donc la TUI
bénéficiera automatiquement de PostgreSQL quand il sera branché.

## Limites assumées
Créations/éditions fines toujours en CLI (formulaires TUI = incrément ultérieur si demandé).
