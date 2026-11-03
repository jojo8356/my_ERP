#!/usr/bin/env bash
# =============================================================================
# installer-prod.sh — clone, compilation « release » et installation de l'ERP
# -----------------------------------------------------------------------------
# Ce script fait exactement ce qu'un déploiement demande, sans rien inventer :
#
#   1. vérifie les prérequis (git, Rust stable, réseau vers github.com) ;
#   2. clone le dépôt (ou réutilise le dossier courant s'il est déjà le dépôt) ;
#   3. compile en release — avec la feature PostgreSQL si demandée ;
#   4. installe le binaire (par défaut /usr/local/bin) ;
#   5. prépare le dossier de données de production et vérifie le binaire.
#
# Il n'écrit **aucune** donnée métier : il ne fait que poser le logiciel.
# Le premier lancement (société, administrateur, clients) reste explicite.
#
# Usage :
#   ./scripts/installer-prod.sh                       # fichiers locaux (TSV)
#   ./scripts/installer-prod.sh --avec-postgres       # + connecteur libpq
#   ./scripts/installer-prod.sh --sans-installation   # compile seulement
#   ./scripts/installer-prod.sh --aide
# =============================================================================
set -euo pipefail

DEPOT="https://github.com/jojo8356/my_ERP.git"
BRANCHE="${ERP_BRANCHE:-main}"
CIBLE="${ERP_PREFIXE:-/usr/local/bin}"
DONNEES="${ERP_DONNEES_DIR:-/var/lib/my-erp}"
AVEC_POSTGRES=0
INSTALLER=1
RACINE=""

VERT=$'\033[38;2;74;222;128m'
ROUGE=$'\033[38;2;255;92;92m'
VIOLET=$'\033[38;2;167;139;250m'
NEUTRE=$'\033[0m'

etape() { printf '\n%s▸ %s%s\n' "$VIOLET" "$*" "$NEUTRE"; }
ok() { printf '  %s✓%s %s\n' "$VERT" "$NEUTRE" "$*"; }
echec() {
  printf '  %s✗%s %s\n' "$ROUGE" "$NEUTRE" "$*" >&2
  exit 1
}
info() { printf '    %s\n' "$*"; }

aide() {
  cat <<'TXT'
installer-prod.sh — installe l'ERP CLI en production

options :
  --depot <url>          dépôt à cloner (défaut : github.com/jojo8356/my_ERP)
  --branche <nom>        branche à construire (défaut : main, ou ERP_BRANCHE)
  --prefixe <dossier>    où installer le binaire (défaut : /usr/local/bin)
  --donnees <dossier>    dossier de données de production (défaut : /var/lib/my-erp)
  --avec-postgres        compile avec la feature postgres (demande libpq-dev)
  --sans-installation    compile et vérifie, sans copier le binaire ni créer le dossier
  --aide                 cette aide

après installation :
  my-erp --version                       # binaire en place
  my-erp /societe modifier --nom …       # identité de l'entreprise
  my-erp /utilisateurs nouveau admin --role admin --mot-de-passe …
  my-erp /utilisateurs 2fa admin         # SEC-02 : obligatoire pour un admin
  my-erp tui                             # usage quotidien (e = société, u = session)
TXT
}

while [ $# -gt 0 ]; do
  case "$1" in
    --aide|-h|--help) aide; exit 0 ;;
    --depot) DEPOT="${2:?}"; shift 2 ;;
    --branche) BRANCHE="${2:?}"; shift 2 ;;
    --prefixe) CIBLE="${2:?}"; shift 2 ;;
    --donnees) DONNEES="${2:?}"; shift 2 ;;
    --avec-postgres) AVEC_POSTGRES=1; shift ;;
    --sans-installation) INSTALLER=0; shift ;;
    *) echec "option inconnue : $1 (voir --aide)" ;;
  esac
done

# ------------------------------------------------------------------ 1. prérequis
etape "Prérequis"
command -v git >/dev/null 2>&1 || echec "git est absent : installez-le (apt install git)"
ok "git $(git --version | awk '{print $3}')"

if ! command -v cargo >/dev/null 2>&1; then
  info "cargo absent — installation de Rust stable via rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
command -v cargo >/dev/null 2>&1 || echec "cargo reste introuvable après installation"
ok "cargo $(cargo --version | awk '{print $2}')"

# Les dépendances chromat et rustorm sont des dépendances **git** : sans accès
# à github.com, la compilation échouerait de façon obscure. On le dit avant.
if ! git ls-remote --heads "$DEPOT" >/dev/null 2>&1; then
  echec "dépôt inaccessible : $DEPOT (vérifiez le réseau et vos droits)"
fi
ok "dépôt joignable : $DEPOT"

if [ "$AVEC_POSTGRES" -eq 1 ]; then
  command -v pg_config >/dev/null 2>&1 ||
    echec "feature postgres demandée : installez libpq-dev (apt install libpq-dev)"
  ok "libpq présent : $(pg_config --version)"
fi

# ------------------------------------------------------------ 2. clone ou local
etape "Code source"
if [ -f "Cargo.toml" ] && grep -q 'name = "my-erp"' Cargo.toml 2>/dev/null; then
  RACINE="$PWD"
  ok "dépôt déjà présent : $RACINE (réutilisé tel quel)"
else
  nom_cible="my_ERP"
  if [ -e "$nom_cible" ]; then
    echec "$nom_cible existe déjà ici — déplacez-le ou lancez le script ailleurs"
  fi
  git clone --depth 1 --branch "$BRANCHE" "$DEPOT" "$nom_cible"
  RACINE="$PWD/$nom_cible"
  ok "cloné dans $RACINE (branche $BRANCHE)"
fi
cd "$RACINE"

# -------------------------------------------------------------- 3. compilation
etape "Compilation release"
if [ "$AVEC_POSTGRES" -eq 1 ]; then
  cargo build --release --features postgres
  BIN="$RACINE/target/release/my-erp"
  ok "binaire release compilé (PostgreSQL activé)"
else
  cargo build --release
  BIN="$RACINE/target/release/my-erp"
  ok "binaire release compilé (mode fichiers, aucune base requise)"
fi
[ -x "$BIN" ] || echec "compilation terminée mais $BIN est absent"
ok "$("$BIN" --version)"

# ------------------------------------------------------------ 4. installation
if [ "$INSTALLER" -eq 1 ]; then
  etape "Installation"
  if [ -w "$CIBLE" ] || [ "$(id -u)" -eq 0 ]; then
    install -m 755 "$BIN" "$CIBLE/my-erp"
  else
    info "$CIBLE n'est pas accessible en écriture : sudo demandé pour la copie"
    sudo install -m 755 "$BIN" "$CIBLE/my-erp"
  fi
  ok "binaire installé : $CIBLE/my-erp"

  command -v my-erp >/dev/null 2>&1 || info "note : $CIBLE n'est pas dans le PATH"
  mkdir -p "$DONNEES" 2>/dev/null || sudo mkdir -p "$DONNEES"
  if [ "$(id -u)" -ne 0 ]; then
    sudo chown "$(id -un)":"$(id -gn)" "$DONNEES" 2>/dev/null || true
  fi
  ok "dossier de données : $DONNEES"
fi

# ---------------------------------------------------------------- 5. vérifier
etape "Vérification"
if [ "$INSTALLER" -eq 1 ]; then
  ERP_DONNEES_DIR="$DONNEES" "$CIBLE/my-erp" /admin sante >/dev/null 2>&1 &&
    ok "état de santé lu sur $DONNEES" ||
    echec "le binaire n'a pas pu lire $DONNEES (droits ?)"
else
  "$BIN" --memoire /admin sante >/dev/null 2>&1 &&
    ok "le binaire démarre (mode mémoire)" ||
    echec "le binaire ne démarre pas"
fi

cat <<TXT

$(printf '%sInstallation terminée.%s' "$VERT" "$NEUTRE")

prochaine étape — identité de l'entreprise et premier administrateur :

  export ERP_DONNEES_DIR="$DONNEES"
  my-erp /societe modifier --nom "Ma société" --siren 123456789 \\
          --adresse "1 rue de la Paix" --cp 06000 --ville Nice --forme micro-entreprise
  my-erp /utilisateurs nouveau admin --role admin --mot-de-passe "…" --email moi@exemple.fr
  my-erp /utilisateurs 2fa admin          # SEC-02 : obligatoire pour un administrateur
  my-erp tui                              # usage quotidien (e = société, u = session)

usage quotidien :
  my-erp /factures --impayees --retard
  my-erp /admin sauvegarde                # sauvegarde datée (SHA-256 vérifiable)
  my-erp /admin audit --verifier          # chaîne d'audit infalsifiable

TXT
