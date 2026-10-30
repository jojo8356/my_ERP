# ADR-004 — Chromat pour toutes les couleurs du terminal

Date : 30/10/2026 · Demandé par l'utilisateur (« utilise ta technologie Chromat pour toutes les print colors, fais ça propre »).

## Décision

Toutes les couleurs affichées par l'ERP passent désormais par
[Chromat](https://github.com/jojo8356/chromat) `v0.1.1`
(dépendance Git épinglée par tag dans `Cargo.toml`), la bibliothèque de
couleurs terminal de l'utilisateur. Chromat est elle-même sans aucune
dépendance et détecte automatiquement le niveau couleur du terminal
(`NO_COLOR`, sortie non-TTY, 16 couleurs, 256 couleurs, truecolor) :
les sorties redirigées vers un fichier ou un pipe restent donc propres,
sans aucun code ANSI — indispensable pour les exports et les scripts.

## Mise en œuvre (« propre »)

- `src/theme.rs` est l'**unique point d'entrée couleur** du projet :
  styles sémantiques `blanc`, `titre` (blanc gras), `violet` (menu),
  `selection` (violet gras), `succes`, `alerte`, `erreur`, `info`.
  Aucun autre module ne choisit de couleur.
- `src/tui.rs` ne contient plus aucun code couleur ANSI écrit à la main ;
  cadre et titre blancs, onglets violets, sélection violette en gras,
  **aucun fond coloré ni surlignage** (style validé par l'utilisateur).
- `src/cli.rs` (tableau de bord, aide, messages `✓`/`⚠`, invite `erp ›`,
  erreurs du REPL) et `src/main.rs` (erreurs fatales, mode mémoire)
  utilisent le même thème.
- Les sorties machine (JSON, FEC, écriture fichier) ne sont **pas**
  colorées : ce ne sont pas des impressions destinées à l'œil.

Restent dans `tui.rs` les seules séquences de *contrôle* du terminal
(écran alternatif, curseur, effacer l'écran) : ce ne sont pas des
couleurs et Chromat ne les gère pas.

## Conséquences

- ADR-002 (« zéro dépendance ») est amendé : l'ERP compte exactement
  **une** dépendance, Chromat, elle-même sans dépendance transitive.
  Le reste demeure en Rust standard (persistance fichier via `Store`,
  PostgreSQL toujours différé au jalon J5).
- ADR-003 est amendé sur la technique d'affichage : les couleurs de la
  TUI viennent de Chromat (ADR-004), les mécanismes terminal restent
  en Rust standard.
- Des tests verrouillent le rendu truecolor de Chromat
  (violet RGB(238, 130, 238), blanc, gras, absence de fond) et son
  comportement sans couleur.

## Précision (ADR-005, 30/10/2026)
Après migration Ratatui, la TUI applique la palette RGB centralisée dans
`theme.rs` à ses propres styles (Ratatui ne consomme pas les chaînes ANSI
de Chromat) ; le CLI continue d'utiliser Chromat directement. Aucun fond
coloré dans les deux cas.
