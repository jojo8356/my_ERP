#!/usr/bin/env bash
# =============================================================================
#  test-all.sh — recette complète de my_ERP, en un seul fichier
# =============================================================================
#
#  Tout ce que la CI exécute, exécutable sur un poste, sans base de données :
#    1. environnement (outils présents, versions)
#    2. contrôles statiques (modèle .rsm, dépendances, couleurs, messages, docs)
#    3. compilation (binaire + tests)
#    4. qualité (rustfmt, clippy) — signalée, jamais bloquante
#    5. tests unitaires et métier (cargo test)
#    6. recette de bout en bout (scénario sur un dossier de données réel :
#       clients, devis, factures, PDF/UBL, e-mails, paiements, Vinted,
#       comptabilité, FEC, TVA, rapprochement, sécurité, multi-société, sauvegarde)
#    7. base PostgreSQL via Rustorm (uniquement si une URL est fournie)
#    8. résumé : une ligne par étape, code de sortie 0 seulement si tout passe
#
#  Usage :
#    ./test-all.sh                        # tout, sans base de données
#    ./test-all.sh --bdd postgres://…     # idem + PostgreSQL (feature postgres)
#    ./test-all.sh --rapide               # sans clippy/rustfmt ni mode release
#    ./test-all.sh --verbose              # garde les sorties détaillées
#    ./test-all.sh --garder               # conserve les dossiers temporaires
#    ERP_DATABASE_URL=postgres://… ./test-all.sh     # équivalent à --bdd
#
#  Aucune dépendance hors de Rust : ni Docker, ni serveur, ni compte.
# =============================================================================
set -uo pipefail

# ---------------------------------------------------------------- Réglages
RACINE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$RACINE" || exit 1

VERBEUX=0
RAPIDE=0
GARDER=0
BASE_URL="${ERP_DATABASE_URL:-}"
FEATURES_EN_PLUS=""
LOGS="$(mktemp -d "${TMPDIR:-/tmp}/erp-tests-XXXXXX")"
DONNEES_TEST="$(mktemp -d "${TMPDIR:-/tmp}/erp-donnees-XXXXXX")"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  VERT=$'\033[32m'; ROUGE=$'\033[31m'; ORANGE=$'\033[33m'
  BLEU=$'\033[36m'; GRAS=$'\033[1m'; NEUTRE=$'\033[0m'
else
  VERT=''; ROUGE=''; ORANGE=''; BLEU=''; GRAS=''; NEUTRE=''
fi

# ---------------------------------------------------------------- Aide
usage() {
  # Tout le bloc de commentaires d'en-tête, sans les « # ».
  sed -n '2,/^set -uo/p' "$0" | grep '^#' | sed 's/^# \{0,1\}//'
  exit 0
}

while [ $# -gt 0 ]; do
  case "$1" in
    --bdd|--base|--database-url)
      BASE_URL="${2:-}"; shift 2 ;;
    --bdd=*|--base=*) BASE_URL="${1#*=}"; shift ;;
    --sans-bdd) BASE_URL=""; shift ;;
    --features) FEATURES_EN_PLUS="${2:-}"; shift 2 ;;
    --features=*) FEATURES_EN_PLUS="${1#*=}"; shift ;;
    --rapide) RAPIDE=1; shift ;;
    -v|--verbose) VERBEUX=1; shift ;;
    --garder) GARDER=1; shift ;;
    -h|--aide|--help) usage ;;
    *) echo "option inconnue : $1 (voir --aide)" >&2; exit 2 ;;
  esac
done

# ---------------------------------------------------------------- Journal
traces=()
etapes_noms=()
etapes_etats=()
etapes_temps=()
ECHECS=0

etape() {
  # etape "<nom>" <fonction>
  local nom="$1" fonction="$2"
  local debut fin duree etat
  debut=$(date +%s)
  printf '\n%s━━━ %s%s\n' "$GRAS" "$nom" "$NEUTRE" >&2
  if "$fonction"; then
    etat="OK"
  else
    etat="ÉCHEC"
    ECHECS=$((ECHECS + 1))
  fi
  fin=$(date +%s)
  duree=$((fin - debut))
  etapes_noms+=("$nom")
  etapes_etats+=("$etat")
  etapes_temps+=("${duree}s")
  if [ "$etat" = "OK" ]; then
    printf '%s  ✓ %s (%ss)%s\n' "$VERT" "$nom" "$duree" "$NEUTRE" >&2
  else
    printf '%s  ✗ %s (%ss) — voir %s%s\n' "$ROUGE" "$nom" "$duree" "$LOGS" "$NEUTRE" >&2
  fi
}

# Exécute une commande et capture sa sortie ; en cas d'échec, la fin du journal
# est affichée (sans noyer un terminal ou un log de CI).
executer() {
  local nom="$1"; shift
  local journal="$LOGS/$(echo "$nom" | tr ' /' '__').log"
  if "$@" >"$journal" 2>&1; then
    [ "$VERBEUX" -eq 1 ] && tail -n 15 "$journal" >&2
    return 0
  fi
  echo "  ⚠ échec : $*" >&2
  tail -n 40 "$journal" >&2
  return 1
}

verifier() {
  # verifier "<libellé>" <commande…>
  local libelle="$1"; shift
  if "$@" >/dev/null 2>&1; then
    printf '  %s✓%s %s\n' "$VERT" "$NEUTRE" "$libelle" >&2
    return 0
  fi
  printf '  %s✗%s %s\n' "$ROUGE" "$NEUTRE" "$libelle" >&2
  return 1
}

nettoyage() {
  if [ "$GARDER" -eq 1 ]; then
    echo "Journaux conservés : $LOGS" >&2
    echo "Données de test conservées : $DONNEES_TEST" >&2
  else
    rm -rf "$LOGS" "$DONNEES_TEST"
  fi
}
trap nettoyage EXIT

# Les binaires compilés sont appelés directement : plus rapide qu'un
# `cargo run` par commande, et surtout sans recompilation surprise.
BIN=""
ERP() {
  if [ -n "$BIN" ]; then "$BIN" "$@"; else cargo run --quiet -- "$@"; fi
}

# =============================================================================
# 1. Environnement
# =============================================================================
etape_environnement() {
  local ok=0
  printf '  racine      : %s\n' "$RACINE" >&2
  if command -v cargo >/dev/null 2>&1; then
    printf '  cargo       : %s\n' "$(cargo --version 2>/dev/null)" >&2
    printf '  rustc       : %s\n' "$(rustc --version 2>/dev/null)" >&2
  else
    printf '  %scargo introuvable : installez Rust (https://rustup.rs)%s\n' "$ROUGE" "$NEUTRE" >&2
    ok=1
  fi
  printf '  git         : %s\n' "$(git rev-parse --short HEAD 2>/dev/null || echo 'hors dépôt')" >&2
  printf '  branche     : %s\n' "$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '-')" >&2
  printf '  rustfmt     : %s\n' "$(command -v rustfmt >/dev/null 2>&1 && echo présent || echo absent)" >&2
  printf '  clippy      : %s\n' "$(cargo clippy --version >/dev/null 2>&1 && echo présent || echo absent)" >&2
  # Jamais d'identifiants dans un journal : on masque la partie « user:secret@ ».
  printf '  base visée  : %s\n' \
    "$(printf '%s' "${BASE_URL:-aucune (mode fichiers)}" | sed -E 's#(://[^:/@]*):[^@]*@#\1:***@#')" >&2
  [ -n "$FEATURES_EN_PLUS" ] && printf '  features    : %s\n' "$FEATURES_EN_PLUS" >&2
  return $ok
}

# =============================================================================
# 2. Contrôles statiques — lisibles sans compiler
# =============================================================================
etape_statique() {
  local echecs=0

  # Fichiers indispensables : le dépôt doit rester auto-portant.
  for f in Cargo.toml build.rs rustorm/schema.rsm CDC-ERP-CLI.md CONFORMITE-CDC.md \
           README.md src/lib.rs src/main.rs src/domain.rs src/fichier.rs src/store.rs \
           src/cli.rs src/orm.rs src/compta.rs src/csv.rs src/pdf.rs src/messagerie.rs \
           src/efacture.rs src/securite.rs src/theme.rs src/tui.rs \
           decisions/ADR-002-persistance-v1.md decisions/ADR-006-rustorm-sans-base.md \
           decisions/ADR-007-securite-locale.md decisions/ADR-008-documents-pdf-email-efacture.md \
           scripts/scenario-demo.sh; do
    verifier "fichier présent : $f" test -s "$f" || echecs=$((echecs + 1))
  done

  # Le schéma est la source de vérité : on compte les modèles déclarés.
  local modeles
  modeles=$(grep -c '^model ' rustorm/schema.rsm)
  if [ "$modeles" -ge 16 ]; then
    printf '  %s✓%s modèle Rustorm : %s tables\n' "$VERT" "$NEUTRE" "$modeles" >&2
  else
    printf '  %s✗%s modèle Rustorm : %s tables (16 attendues)\n' "$ROUGE" "$NEUTRE" "$modeles" >&2
    echecs=$((echecs + 1))
  fi

  # Chaque module déclaré dans lib.rs doit exister sur le disque.
  local module manquant=0
  for module in $(grep -oE '^pub mod [a-z_]+' src/lib.rs | awk '{print $3}'); do
    if [ ! -f "src/$module.rs" ]; then
      printf '  %s✗%s module déclaré sans fichier : src/%s.rs\n' "$ROUGE" "$NEUTRE" "$module" >&2
      manquant=1
    fi
  done
  if [ $manquant -eq 0 ]; then
    printf '  %s✓%s tous les modules de lib.rs existent\n' "$VERT" "$NEUTRE" >&2
  else
    echecs=$((echecs + 1))
  fi

  # Aucune séquence ANSI littérale hors de theme.rs / tui.rs (ADR-004).
  if grep -rlP '\x1b' src --include='*.rs' 2>/dev/null | grep -vE 'src/(theme|tui)\.rs' >/dev/null; then
    printf '  %s✗%s couleur ANSI écrite hors de theme.rs :\n' "$ROUGE" "$NEUTRE" >&2
    grep -rlP '\x1b' src --include='*.rs' | grep -vE 'src/(theme|tui)\.rs' >&2
    echecs=$((echecs + 1))
  else
    printf '  %s✓%s aucune couleur ANSI hors de theme.rs / tui.rs\n' "$VERT" "$NEUTRE" >&2
  fi

  # Messages en français (ENF-ERG-01) : les erreurs des modules métier sont
  # rédigées en français, pas en anglais technique.
  if grep -rnE 'Err\("(error|cannot|failed|invalid|unsupported)' src --include='*.rs' >/dev/null; then
    printf '  %s✗%s message d’erreur en anglais détecté\n' "$ROUGE" "$NEUTRE" >&2
    grep -rnE 'Err\("(error|cannot|failed|invalid)' src --include='*.rs' >&2
    echecs=$((echecs + 1))
  else
    printf '  %s✓%s messages d’erreur rédigés en français\n' "$VERT" "$NEUTRE" >&2
  fi

  # Le socle « sans base de données » doit rester vrai par défaut.
  if grep -qE '^postgres = \["rustorm/postgres"\]' Cargo.toml; then
    printf '  %s✓%s feature postgres déclarée mais désactivée par défaut\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s la feature postgres doit rester optionnelle (ADR-009)\n' "$ROUGE" "$NEUTRE" >&2
    echecs=$((echecs + 1))
  fi

  # Scripts : syntaxe bash vérifiée avant de les exécuter.
  local script
  for script in test-all.sh scripts/*.sh; do
    [ -f "$script" ] || continue
    if bash -n "$script" 2>>"$LOGS/bash.log"; then
      printf '  %s✓%s syntaxe bash : %s\n' "$VERT" "$NEUTRE" "$script" >&2
    else
      printf '  %s✗%s syntaxe bash : %s\n' "$ROUGE" "$NEUTRE" "$script" >&2
      echecs=$((echecs + 1))
    fi
  done

  [ "$echecs" -eq 0 ]
}

# =============================================================================
# 3. Compilation
# =============================================================================
etape_compilation() {
  local features_arg=()
  [ -n "$FEATURES_EN_PLUS" ] && features_arg=(--features "$FEATURES_EN_PLUS")
  executer "compilation" cargo build --all-targets ${features_arg[@]+"${features_arg[@]}"} || return 1
  BIN="$RACINE/target/debug/my-erp"
  verifier "binaire produit : target/debug/my-erp" test -x "$BIN" || return 1
  if [ "$RAPIDE" -eq 0 ]; then
    executer "compilation-release" cargo build --release ${features_arg[@]+"${features_arg[@]}"} || return 1
    verifier "binaire release produit" test -x "$RACINE/target/release/my-erp"
  fi
}

# =============================================================================
# 4. Qualité (signalée, jamais bloquante : le dépôt n'est pas formaté « rustfmt »
#    à la lettre et les warnings du client généré ne nous appartiennent pas)
# =============================================================================
etape_qualite() {
  if cargo fmt --version >/dev/null 2>&1; then
    if cargo fmt --all -- --check >"$LOGS/fmt.log" 2>&1; then
      printf '  %s✓%s rustfmt : code conforme\n' "$VERT" "$NEUTRE" >&2
    else
      printf '  %s!%s rustfmt : différences de formatage (%s lignes) — non bloquant\n' \
        "$ORANGE" "$NEUTRE" "$(wc -l <"$LOGS/fmt.log")" >&2
    fi
  else
    printf '  %s!%s rustfmt absent : contrôle ignoré\n' "$ORANGE" "$NEUTRE" >&2
  fi

  if cargo clippy --version >/dev/null 2>&1; then
    cargo clippy --all-targets --quiet -- -A clippy::all >"$LOGS/clippy.log" 2>&1
    local series
    series=$(grep -c '^error' "$LOGS/clippy.log" 2>/dev/null || true)
    if [ "${series:-0}" -eq 0 ]; then
      printf '  %s✓%s clippy : aucune erreur bloquante\n' "$VERT" "$NEUTRE" >&2
    else
      printf '  %s!%s clippy : %s erreur(s) — voir %s\n' "$ORANGE" "$NEUTRE" "$series" "$LOGS/clippy.log" >&2
      tail -n 15 "$LOGS/clippy.log" >&2
    fi
  else
    printf '  %s!%s clippy absent : contrôle ignoré\n' "$ORANGE" "$NEUTRE" >&2
  fi
  return 0
}

# =============================================================================
# 5. Tests unitaires et métier
# =============================================================================
etape_tests() {
  executer "tests" cargo test || return 1
  local total
  total=$(grep -hoE 'test result: ok\. [0-9]+ passed' "$LOGS/tests.log" 2>/dev/null |
          grep -oE '^[0-9]+|test result: ok\. [0-9]+' | grep -oE '[0-9]+' |
          awk '{s += $1} END {print s + 0}')
  printf '  %s✓%s %s tests verts (unitaires + doc)\n' "$VERT" "$NEUTRE" "${total:-?}" >&2
  # Les règles de calcul doivent rester couvertes : on vérifie la présence des
  # modules critiques dans la sortie des tests.
  local module
  for module in domain compta store cli orm csv pdf messagerie efacture securite tui fichier bdd; do
    if grep -q "^test ${module}::" "$LOGS/tests.log" 2>/dev/null; then
      printf '  %s✓%s tests du module %s exécutés\n' "$VERT" "$NEUTRE" "$module" >&2
    else
      printf '  %s!%s aucun test exécuté pour %s\n' "$ORANGE" "$NEUTRE" "$module" >&2
    fi
  done
  # Aucun test ignoré ne doit dépendre d'un service externe sans le dire.
  if grep -qE '[1-9][0-9]* ignored' "$LOGS/tests.log"; then
    printf '  %s!%s des tests sont ignorés (base de données : voir --bdd)\n' "$ORANGE" "$NEUTRE" >&2
  fi
  return 0
}

# =============================================================================
# 6. Recette de bout en bout sur un dossier de données réel
# =============================================================================
etape_recette() {
  export ERP_DONNEES_DIR="$DONNEES_TEST/fichiers"
  # La recette fonctionnelle valide le **mode fichiers** : même si une base est
  # configurée pour l'étape 7, on ne la laisse pas s'inviter ici.
  unset ERP_DATABASE_URL
  rm -rf "$ERP_DONNEES_DIR"
  mkdir -p "$ERP_DONNEES_DIR/exports"

  echo "  → scénario complet (scripts/scenario-demo.sh)" >&2
  # On impose le binaire déjà compilé (avec les features demandées) : `cargo run`
  # en recompilerait un autre, sans les features, et écraserait celui-ci.
  if ! ERP_BIN="$BIN" ERP_DONNEES_DIR="$ERP_DONNEES_DIR" \
       bash scripts/scenario-demo.sh >"$LOGS/recette.log" 2>&1; then
    tail -n 40 "$LOGS/recette.log" >&2
    return 1
  fi
  grep -E '^(──|    )' "$LOGS/recette.log" | sed 's/^/  /' >&2

  # Vérifications de sortie, indépendantes du scénario lui-même.
  local echecs=0
  verifier "persistance : meta.tsv écrit" test -s "$ERP_DONNEES_DIR/meta.tsv" || echecs=$((echecs+1))
  verifier "persistance : tables TSV écrites" \
    test -n "$(ls "$ERP_DONNEES_DIR"/tables/*.tsv 2>/dev/null)" || echecs=$((echecs+1))
  # Les métadonnées vivent dans `meta.tsv` à la racine, pas dans `tables/`.
  local tables_attendues
  tables_attendues=$(grep -c '^model ' rustorm/schema.rsm)
  tables_attendues=$((tables_attendues - 1))
  verifier "une table TSV par modèle du schéma ($tables_attendues)" \
    test "$(ls "$ERP_DONNEES_DIR"/tables/*.tsv 2>/dev/null | wc -l)" -eq "$tables_attendues" || echecs=$((echecs+1))

  # Sorties machine : JSON et CSV doivent rester exploitables par un script.
  if "$BIN" /clients --json 2>/dev/null | grep -q '^\['; then
    printf '  %s✓%s sortie JSON des listes\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s sortie JSON des listes\n' "$ROUGE" "$NEUTRE" >&2; echecs=$((echecs+1))
  fi
  if "$BIN" /factures --csv 2>/dev/null | head -1 | grep -q 'numero'; then
    printf '  %s✓%s export CSV des listes\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s export CSV des listes\n' "$ROUGE" "$NEUTRE" >&2; echecs=$((echecs+1))
  fi

  # Pagination : jamais 500 lignes d'un coup (CDC §6).
  local i
  for i in $(seq 1 25); do
    "$BIN" /clients nouveau --nom "Client $i" --type particulier >/dev/null 2>&1
  done
  if "$BIN" /clients --lignes 10 2>/dev/null | grep -q 'page 1/'; then
    printf '  %s✓%s pagination des longues listes\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s pagination des longues listes\n' "$ROUGE" "$NEUTRE" >&2; echecs=$((echecs+1))
  fi

  # Aide contextuelle et messages actionnables en français.
  if "$BIN" /factures 2>/dev/null | grep -q 'FA-' && "$BIN" /aide factures 2>/dev/null | grep -q '/factures'; then
    printf '  %s✓%s aide contextuelle\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s aide contextuelle\n' "$ROUGE" "$NEUTRE" >&2; echecs=$((echecs+1))
  fi
  # Un chemin inconnu doit échouer (code ≠ 0) **et** expliquer quoi faire.
  "$BIN" /chemin/inexistant >/dev/null 2>"$LOGS/erreur.log"
  code=$?
  if [ "$code" -ne 0 ] && grep -q 'chemin inconnu' "$LOGS/erreur.log"; then
    printf '  %s✓%s chemin inconnu : refus explicite et code de sortie %s\n' "$VERT" "$NEUTRE" "$code" >&2
  else
    printf '  %s✗%s chemin inconnu : message ou code de sortie inattendu (%s)\n' "$ROUGE" "$NEUTRE" "$code" >&2
    echecs=$((echecs+1))
  fi

  # Audit infalsifiable : la chaîne doit être vérifiable après tous ces ordres.
  if "$BIN" /admin audit --verifier 2>/dev/null | grep -q 'aucune altération'; then
    printf '  %s✓%s chaîne d’audit vérifiée\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s chaîne d’audit\n' "$ROUGE" "$NEUTRE" >&2; echecs=$((echecs+1))
  fi

  # Migration PostgreSQL générée hors ligne, sans serveur.
  if "$BIN" /admin schema --sql 2>/dev/null | grep -q 'CREATE TABLE "clients"'; then
    printf '  %s✓%s migration PostgreSQL produite hors ligne\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s migration PostgreSQL\n' "$ROUGE" "$NEUTRE" >&2; echecs=$((echecs+1))
  fi

  unset ERP_DONNEES_DIR
  [ "$echecs" -eq 0 ]
}

# =============================================================================
# 7. PostgreSQL via Rustorm (facultatif)
# =============================================================================
etape_base() {
  if [ -z "$BASE_URL" ]; then
    printf '  %s!%s aucune base fournie (--bdd postgres://… ou ERP_DATABASE_URL) : étape ignorée\n' \
      "$ORANGE" "$NEUTRE" >&2
    return 0
  fi
  local echecs=0
  local ref="$LOGS/base.log"

  # 7.1 le binaire doit inclure la feature postgres
  if ! "$RACINE/target/debug/my-erp" --version >/dev/null 2>&1; then
    printf '  %s✗%s binaire absent : compilez d’abord (étape 3)\n' "$ROUGE" "$NEUTRE" >&2
    return 1
  fi
  # Le binaire doit avoir été compilé avec la feature ; sinon `/admin bdd` le dit
  # explicitement (et l'étape ne peut pas se poursuivre).
  if "$BIN" /admin bdd 2>&1 | grep -q 'sans la feature'; then
    printf '  %s✗%s binaire compilé sans la feature postgres\n' "$ROUGE" "$NEUTRE" >&2
    printf '        relancez : %s --features postgres%s\n' "$0" "" >&2
    return 1
  fi
  printf '  %s✓%s binaire compilé avec le connecteur PostgreSQL de Rustorm\n' "$VERT" "$NEUTRE" >&2

  # 7.2 migration par l'ORM puis état de la base
  if "$BIN" --bdd "$BASE_URL" /admin bdd >"$ref" 2>&1; then
    printf '  %s✓%s connexion et migration (Rustorm) :\n' "$VERT" "$NEUTRE" >&2
    sed 's/^/    /' "$ref" >&2
  else
    printf '  %s✗%s connexion à la base\n' "$ROUGE" "$NEUTRE" >&2
    tail -n 20 "$ref" >&2
    return 1
  fi

  # 7.3 tous les modèles doivent exister côté serveur
  local table
  for table in $(grep -oE '@@map\("[a-z_]+"\)' rustorm/schema.rsm | sed 's/@@map("//; s/")//'); do
    if grep -qE "^    $table +[0-9]+ ligne" "$ref"; then
      printf '  %s✓%s table %s présente en base\n' "$VERT" "$NEUTRE" "$table" >&2
    else
      printf '  %s✗%s table %s absente de la base\n' "$ROUGE" "$NEUTRE" "$table" >&2
      echecs=$((echecs+1))
    fi
  done

  # 7.4 parcours réel : écrire, fermer, rouvrir (la mémoire ne peut pas tricher)
  local base_essai="${BASE_URL}"
  local suffixe="test-all-$$"
  "$BIN" --bdd "$base_essai" /admin bdd vider >/dev/null 2>>"$ref"
  "$BIN" --bdd "$base_essai" /societe modifier --nom "Recette SQL" \
      --siren 123456789 --adresse "1 rue de la Paix" --ville Nice >>"$ref" 2>&1
  "$BIN" --bdd "$base_essai" /clients nouveau --nom "$suffixe" --type pro >>"$ref" 2>&1
  "$BIN" --bdd "$base_essai" /factures nouvelle --client "$suffixe" \
      --ligne "Maintenance;2;500,00;2000" >>"$ref" 2>&1
  local numero
  numero=$("$BIN" --bdd "$base_essai" /factures --csv 2>/dev/null | sed -n '2p' | cut -d';' -f1)
  if [ -z "$numero" ]; then
    printf '  %s✗%s écriture d’une facture en base\n' "$ROUGE" "$NEUTRE" >&2
    tail -n 20 "$ref" >&2
    return 1
  fi
  "$BIN" --bdd "$base_essai" /factures "$numero" valider >>"$ref" 2>&1
  "$BIN" --bdd "$base_essai" /factures "$numero" payer >>"$ref" 2>&1

  if "$BIN" --bdd "$base_essai" /factures "$numero" 2>/dev/null | grep -qE 'payée|paye'; then
    printf '  %s✓%s facture %s validée, payée, relue depuis la base\n' "$VERT" "$NEUTRE" "$numero" >&2
  else
    printf '  %s✗%s relecture de la facture depuis la base\n' "$ROUGE" "$NEUTRE" >&2
    echecs=$((echecs+1))
  fi
  if "$BIN" --bdd "$base_essai" /clients 2>/dev/null | grep -q "$suffixe"; then
    printf '  %s✓%s client relu depuis la base après fermeture du processus\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s relecture du client\n' "$ROUGE" "$NEUTRE" >&2
    echecs=$((echecs+1))
  fi
  if "$BIN" --bdd "$base_essai" /compta balance 2>/dev/null | grep -q '411000'; then
    printf '  %s✓%s comptabilité générée en mode base\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s comptabilité en mode base\n' "$ROUGE" "$NEUTRE" >&2
    echecs=$((echecs+1))
  fi

  # 7.5 tests d'intégration Rust (ignorés par défaut, exécutés ici)
  if ERP_DATABASE_URL="$BASE_URL" cargo test --features postgres -- --ignored \
       >"$LOGS/tests-bdd.log" 2>&1; then
    printf '  %s✓%s tests d’intégration PostgreSQL (%s)\n' "$VERT" "$NEUTRE" \
      "$(grep -hoE 'test result: ok\. [0-9]+ passed' "$LOGS/tests-bdd.log" | tail -1)" >&2
  else
    printf '  %s✗%s tests d’intégration PostgreSQL\n' "$ROUGE" "$NEUTRE" >&2
    tail -n 30 "$LOGS/tests-bdd.log" >&2
    echecs=$((echecs+1))
  fi

  # 7.6 retour au mode fichiers : la même commande, sans base
  if ERP_DONNEES_DIR="$DONNEES_TEST/retour" "$BIN" /clients 2>/dev/null >/dev/null; then
    printf '  %s✓%s le mode fichiers reste indépendant de la base\n' "$VERT" "$NEUTRE" >&2
  else
    printf '  %s✗%s mode fichiers après la base\n' "$ROUGE" "$NEUTRE" >&2
    echecs=$((echecs+1))
  fi

  [ "$echecs" -eq 0 ]
}

# =============================================================================
# 8. Résumé
# =============================================================================
resume() {
  printf '\n%s══════════════════════════════════════════════════════════%s\n' "$BLEU" "$NEUTRE" >&2
  printf '%s  Résumé de la recette — my_ERP %s%s\n' "$GRAS" "$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)" "$NEUTRE" >&2
  printf '%s══════════════════════════════════════════════════════════%s\n' "$BLEU" "$NEUTRE" >&2
  local i
  for i in "${!etapes_noms[@]}"; do
    if [ "${etapes_etats[$i]}" = "OK" ]; then
      printf '  %s✓%s %-28s %s\n' "$VERT" "$NEUTRE" "${etapes_noms[$i]}" "${etapes_temps[$i]}" >&2
    else
      printf '  %s✗%s %-28s %s\n' "$ROUGE" "$NEUTRE" "${etapes_noms[$i]}" "${etapes_temps[$i]}" >&2
    fi
  done
  printf '\n' >&2
  if [ "$ECHECS" -eq 0 ]; then
    printf '  %s✅ TOUT EST VERT%s — %s étape(s), base : %s\n\n' "$VERT$GRAS" "$NEUTRE" \
      "${#etapes_noms[@]}" "${BASE_URL:+PostgreSQL (Rustorm)}${BASE_URL:-non utilisée (mode fichiers)}" >&2
    return 0
  fi
  printf '  %s❌ %s étape(s) en échec%s — journaux : %s\n\n' "$ROUGE$GRAS" "$ECHECS" "$NEUTRE" "$LOGS" >&2
  return 1
}

# =============================================================================
# Déroulé
# =============================================================================
printf '%s╔══════════════════════════════════════════════════════════╗%s\n' "$BLEU" "$NEUTRE" >&2
printf '%s║  my_ERP — recette complète (aucune GUI, base optionnelle)  ║%s\n' "$BLEU" "$NEUTRE" >&2
printf '%s╚══════════════════════════════════════════════════════════╝%s\n' "$BLEU" "$NEUTRE" >&2

etape "1. Environnement" etape_environnement
etape "2. Contrôles statiques" etape_statique
etape "3. Compilation" etape_compilation
[ "$RAPIDE" -eq 0 ] && etape "4. Qualité du code" etape_qualite
etape "5. Tests unitaires" etape_tests
etape "6. Recette fonctionnelle" etape_recette
etape "7. Base PostgreSQL" etape_base

resume
