# ADR-005 — Migration TUI vers Ratatui + formulaires interactifs

Date : 30/10/2026 · Demandé par l'utilisateur (« créer des formulaires du coup avec Ratatui, fais la migration »).

## Décision

La TUI est migrée de l'ANSI maison vers **Ratatui 0.29 + Crossterm 0.28** :

- Ratatui rend l'écran (titre, onglets, listes, zones centrées) dans un
  buffer ; Crossterm gère le mode raw, l'écran alternatif et les événements
  clavier/souris/redimensionnement, de façon multiplateforme.
- Les anciens rendus textuels maison, `stty` et le lecteur de touches
  écrit à la main sont supprimés.

## Formulaires (cœur de la demande)

Chaque création/action importante devient un **formulaire plein écran**
éditable au clavier, ouvert :

- par la touche **`f`** : menu de tous les formulaires ;
- par la touche **`n`** : formulaire adapté au module courant ;
- par des touches contextuelles : `d` devis, `p` paiement (dans Factures),
  `Entrée` vente de l'article sélectionné (Vinted), `s` société et
  `x` export FEC (Comptabilité).

Formulaires livrés : **nouveau client, nouvelle facture, nouveau devis,
paiement d'une facture, nouvel article Vinted, vente d'un article,
régime TVA de la société, export FEC**. Navigation : Tab/↑↓/Entrée entre
champs, ←→ pour les champs à choix (type, activité, moyen de paiement…),
Ctrl+S ou Entrée sur le dernier champ pour enregistrer, Échap pour
annuler. La validation réutilise les règles du domaine et du `Store`
(composition textile obligatoire, client existant, montants positifs…).
En cas d'erreur, le formulaire reste ouvert, affiche le motif et
**rien n'est écrit** ; en cas de succès, l'écran est rafraîchi et un
message de confirmation s'affiche. Chaque envoi est journalisé.

## Couleurs

Ratatui utilise ses propres styles : il ne consomme pas les chaînes ANSI
de Chromat. La palette RGB reste centralisée dans `src/theme.rs`
(constantes `*_RGB`) et la TUI l'applique avec le style validé :
cadre/titre blancs et sélection violette lisible. Les onglets suivent
les règles classiques (Material Design, Nielsen Norman Group) :
colonnes égales, libellés centrés, pas de séparateurs verticaux durs,
inactifs atténués à 70 %, et sélection signalée par deux indicateurs —
gras + léger surlignage blanc 20 % limité au libellé (RGB 51,51,51 sur
fond noir, le terminal ne gérant pas la transparence) et soulignement
violet. Le premier essai (bloc surligné géant avec padding 6/2) a été
corrigé le 30/10/2026 : trop massif, il brisait l'alignement. Le CLI
continue d'utiliser directement Chromat (ADR-004).

## Conséquences

- Dépendances : Chromat (Git, sans dépendance propre), Ratatui et
  Crossterm (crates.io, accessibles en CI). La compilation reste rapide
  et l'application une TUI terminal, sans GUI.
- `generer_fec` est extrait dans `store.rs` pour être partagé entre le
  CLI (`/compta fec`) et le formulaire d'export de la TUI.
- ADR-003 est remplacé par le présent ADR pour la technique TUI ;
  ADR-004 reste valable pour le CLI, avec la palette partagée ci-dessus.

## Limites assumées

Formulaires à une ligne de document (désignation/quantité/prix/TVA) ;
les documents multi-lignes restent créables en CLI. Édition d'objets
existants : via les actions dédiées (valider, payer, vendre) ; pas
encore de formulaire d'édition complète.

## Correction palette (30/10/2026)
Le vert CSS de base (0,128,0) utilisé pour les succès était trop sombre
et saturé pour le fond noir : il vibrait contre le violet. Conformément
aux règles des thèmes sombres (variantes plus claires et désaturées des
couleurs sémantiques, luminosité homogène, contraste lisible), toute
l'échelle sémantique de `theme.rs` passe en tons « 400 » : succès
#4ADE80, alerte #FBBF24, erreur #F87171, info #60A5FA — le vert clair
reste le complémentaire naturel du violet, sans l'écraser.
