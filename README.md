# my_ERP

ERP sur mesure — le projet démarre par le **choix de la stack**, avec la méthode des grandes entreprises
(matrice de décision pondérée : besoin métier, scalabilité, équipe/recrutement, sécurité, TCO).

## 🗳️ Première étape : répondez au questionnaire de décision

Ouvrez **`index.html`** (artefact interactif) :

- 13 sections adaptées à un ERP : contexte, utilisateurs, modules, architecture, frontend, backend,
  données, sécurité/RGPD, intégrations, hébergement/DevOps, équipe/budget, arbitrages.
- 💾 Sauvegarde automatique dans le navigateur, reprise à tout moment.
- 🧭 Recommandation de stack générée en direct à partir de vos réponses.
- 📤 Export **JSON** (`mes-choix.json`) ou **Markdown** à transmettre à l'assistant pour enregistrement.

Version texte : [`CHOIX-STACK-ERP.md`](CHOIX-STACK-ERP.md) · Vos réponses versionnées : [`decisions/`](decisions/)

## 📄 Cahier des charges

**[CDC-ERP-CLI.md](CDC-ERP-CLI.md) · **lecture confort : [CDC.html](CDC.html)** (ouvrir dans un navigateur : sommaire, recherche, mode sombre)** — CDC complet v1.0 (ERP CLI Rust + PostgreSQL, conformité services informatiques & Vinted), en attente de validation (réponses au §20 + GO J0).

## Servir l'artefact en local

```bash
python3 -m http.server 8000
# puis http://localhost:8000
```

## 🖥️ V1 — ERP CLI en Rust (livrée, CI verte)

Zéro GUI, **aucune base de données**. Couleurs : [Chromat](https://github.com/jojo8356/chromat)
(bibliothèque de l’auteur, ADR-004) pilote le CLI via `src/theme.rs` ; la TUI
utilise **Ratatui + Crossterm** (ADR-005) avec la même palette et des
**formulaires** pour chaque création/action. Le modèle de données est un schéma
[**Rustorm**](https://github.com/jojo8356/rustorm) (`rustorm/schema.rsm`, 17 tables)
validé **et compilé à chaque build** : le client typé, les colonnes des fichiers
de données et la future migration PostgreSQL viennent tous du même fichier
(ADR-006). Aucune connexion n’est ouverte : le stockage local est un moteur
tabulaire piloté par le schéma.

Navigation par **chemins type URL** et raccourcis, comme exigé dans le
[CDC](CDC-ERP-CLI.md). Périmètre couvert : CRM, devis/factures/avoirs, PDF
conforme + e-facture UBL/EN 16931, envoi e-mail (RFC 5322), relances graduées,
comptabilité (écritures, TVA, FEC, balance, rapprochement bancaire), seuils
micro-entreprise, achat-revente Vinted (marges, rotation, alerte DAC7),
prospects et clients, multi-sociétés, multi-devises, utilisateurs/rôles/2FA (application TOTP **ou**
code à usage unique envoyé par e-mail), et journal d’audit chaîné.
Le détail exigence par exigence est dans **[CONFORMITE-CDC.md](CONFORMITE-CDC.md)**.

```bash
cargo build --release            # binaire unique : target/release/my-erp
./target/release/my-erp          # mode interactif (erp:/chemin>), tapez « aide »
./target/release/my-erp tui      # TUI : ←→ onglets, ↑↓ défiler, n nouveau, p paiement, s recherche, ? aide, q quitter
./target/release/my-erp /clients nouveau --nom "Atelier Dupont" --type pro
./target/release/my-erp /factures --impayees --retard      # commande unique, scriptable
./target/release/my-erp /factures FA-2026-0001 pdf facture.pdf
./target/release/my-erp /compta fec > fec.csv              # FEC DGFiP
./target/release/my-erp /admin schema --sql                # migration PostgreSQL prête
./target/release/my-erp /utilisateurs code admin           # 2e facteur par e-mail (SEC-03)
bash scripts/scenario-demo.sh    # recette complète commentée (16 étapes)
```

Sorties machine : `--json` et `--csv` sur toutes les listes, pagination
`--page`/`--lignes`, `NO_COLOR` respecté, code de sortie 0/1.

### Où sont les données

```
donnees/
  meta.tsv               société courante, session, compteur d'identifiants, version
  tables/*.tsv           une table du schéma = un fichier (colonnes = celles du .rsm)
  courriels/*.eml        messages générés (facture, devis, relance, code de connexion)
  sauvegardes/           copies datées (empreinte SHA-256 vérifiable)
```

Dossier modifiable via `ERP_DONNEES_DIR` ou `--donnees <dossier>` ; `--memoire`
= démonstration sans écriture disque. L’ancien `etat.tsv` de la v1 est importé
automatiquement au premier démarrage (ENF-PORT-01). Sauvegarde = `/admin
sauvegarde` (ou copie du dossier) ; restauration = `/admin restaurer <nom>`.

### Prospects et clients

Une fiche naît **prospect** (piste commerciale) ou **client** ; les deux se
créent de la même façon, le statut se change ensuite en une commande :

```bash
./target/release/my-erp /clients nouveau --nom "Piste Médicale" --type pro --prospect
./target/release/my-erp /prospects                       # alias de /clients --prospects
./target/release/my-erp /clients "piste médicale"        # fiche : statut + dernier contact
./target/release/my-erp /devis nouvelle --client piste --ligne "Audit;1;900,00"
./target/release/my-erp /clients piste convertir         # prospect → client
./target/release/my-erp /clients piste prospect          # retour en piste
```

**Notes datées** : chaque fiche tient un journal (`/clients <nom> note "…"`,
`/clients <nom> notes`), signé par la session et conservé avec la fiche — c'est
le suivi commercial, distinct du champ libre `--notes`.

Dans la **TUI** : `c` ouvre l'interrupteur de statut (Espace bascule, Entrée
valide, état affiché `[●] client / [○] prospect`), `o` ajoute une note.

Un prospect peut recevoir des **devis** ; la **facture** est refusée tant que la
fiche n'est pas convertie (message qui donne la commande exacte). Filtres :
`--prospects`, `--clients`, `--perimes` (RGPD-02 : prospects sans contact depuis
plus de 3 ans, à anonymiser ou convertir). Le statut apparaît dans la liste, la
fiche, le CSV et le JSON, et le formulaire de la TUI propose « client » par défaut.

### Plusieurs sociétés, plusieurs sessions — un sélecteur dans la TUI

Les données sont **cloisonnées par société** (clients, documents, compta…) et la
société courante comme la session sont enregistrées dans `meta.tsv` : on
retrouve son contexte au redémarrage, sans variable d'environnement.

```
TUI   e   sélecteur de sociétés   ▸ = société courante, ↑↓ pour choisir, Entrée bascule
TUI   u   sélecteur de sessions   les comptes + « Session locale » (déconnexion)
CLI       /societes basculer <nom>   ·   /societes/<nom>   ·   --societe <nom>
CLI       /auth connexion <compte> --mot-de-passe … [--totp 123456]
```

Le mode interactif rappelle le contexte dans son invite — impossible de se
tromper d'entreprise sans le voir :

```
erp [Atelier Dupont · jojo] › /factures
```

Le sélecteur de session ouvre la **connexion du compte choisi** : le mot de passe
est saisi **masqué** (`••••`, jamais affiché, jamais journalisé) et le code 2FA
reste facultatif — les règles d'authentification sont celles du CLI, la TUI ne
les réimplémente pas. L'en-tête rappelle en permanence société, régime, devise et
session, pour ne jamais saisir une facture dans la mauvaise entreprise.

### Retrouver et emporter ses données

Une seule question à se poser : **où est l'état, et comment le sortir ?**

```bash
./target/release/my-erp /admin chemin          # dossier absolu, une ligne par table, tailles
./target/release/my-erp /admin chemin --json   # même chose pour un script/monitoring
./target/release/my-erp /admin export csv      # TOUTES les tables → CSV + meta.csv + manifeste.csv
./target/release/my-erp /admin export tsv --dossier /media/usb/erp
./target/release/my-erp /admin export bdd postgres://erp@127.0.0.1:5432/erp
```

- `/admin chemin` donne le chemin **exact** (canonique) du dossier `donnees/` ou,
  en mode base, l'URL PostgreSQL (mot de passe masqué) : c'est la « route » à
  mettre dans `cp`, `rsync`, une unité systemd ou `pg_dump` — les commandes sont
  affichées prêtes à copier.
- `/admin export csv|tsv` écrit **une table du schéma = un fichier**, avec
  `meta.csv` (société courante, session, version, empreinte du schéma) et
  `manifeste.csv` (table, lignes, colonnes, fichier) : de quoi tout recharger
  dans un tableur, un script ou un autre outil. Défaut :
  `${ERP_DONNEES_DIR}/exports/export-<horodatage>/`, sinon `--dossier`.
- `/admin export bdd <url>` est la **migration** fichiers → PostgreSQL : le
  schéma est créé par l'ORM si besoin, puis toutes les tables sont écrites dans
  une transaction. Le dossier local reste intact (`--bdd` ensuite pour lire la
  base, `ERP_DATABASE_URL` pour ne plus y penser).
- Sens inverse (base → fichiers) : `pg_dump` (cité par `/admin chemin`) ou
  `/admin export csv` depuis une session ouverte sur la base.

### Deux règles de conformité appliquées, pas seulement documentées

- **Facture obligatoire ou non (CDC §11.2)** — `TypeClient::facture_obligatoire` :
  professionnel, toujours ; particulier, dès **25 € TTC** ou sur demande. La règle
  est rappelée à chaque création de facture (et à la transformation d’un devis) ;
  sous le seuil, l’ERP signale qu’une note suffit, sans jamais l’interdire :

  ```bash
  ./target/release/my-erp /factures nouvelle --client martin --ligne "Dépannage;1;10,00"
  #   ⚖ facture facultative : particulier sous 25,00 € TTC (montant 10,00 €) — une note
  #     suffit, ajoutez --sur-demande si le client la réclame (CDC §11.2)
  ```

- **Code de connexion par e-mail (SEC-03)** — en plus du TOTP, un **code à usage
  unique** peut être envoyé au compte ; seule son empreinte est stockée (15 min,
  5 tentatives), et le code ne s’affiche jamais à l’écran :

  ```bash
  ./target/release/my-erp /utilisateurs email admin --email moi@exemple.fr
  ./target/release/my-erp /utilisateurs code admin      # → données/courriels/*.eml
  ./target/release/my-erp /auth connexion admin --mot-de-passe … --code 123456
  ```

### Base de données PostgreSQL (optionnelle, par Rustorm)

Le mode par défaut ne demande **aucun serveur**. Le même binaire sait aussi
travailler sur PostgreSQL : c’est Rustorm qui crée le schéma (migration), lit et
écrit les tables, et regroupe chaque enregistrement dans une transaction
(ADR-009). Toutes les commandes restent identiques.

```bash
sudo apt install libpq-dev        # connecteur libpq (une seule fois)
cargo build --features postgres
export ERP_DATABASE_URL=postgres://erp:erp@127.0.0.1:5432/erp
./target/debug/my-erp /admin bdd            # support, tables et lignes réelles
./target/debug/my-erp /clients              # mêmes commandes, état dans la base
./target/debug/my-erp --bdd "$ERP_DATABASE_URL" /admin bdd vider
./target/debug/my-erp /admin schema --sql   # la migration exacte, hors ligne
```

Sans `--features postgres`, l’option `--bdd` répond explicitement qu’il faut
recompiler : aucune dépendance à `libpq` n’est imposée au mode fichiers.

### Tester tout, en une commande

```bash
./test-all.sh                                  # 7 étapes, sans base de données
./test-all.sh --bdd postgres://erp:erp@127.0.0.1/erp   # + PostgreSQL (Rustorm)
./test-all.sh --rapide --verbose               # itération locale
```

`test-all.sh` enchaîne : contrôle de l’environnement, contrôles statiques du
modèle et des modules, compilation, rustfmt/clippy (signalés, non bloquants),
tests unitaires, **recette fonctionnelle complète** (clients, devis, factures,
PDF/UBL, e-mails, paiements, Vinted, comptabilité, FEC, TVA, rapprochement,
sécurité/2FA, multi-société, sauvegarde, pagination, JSON/CSV, audit), puis la
base PostgreSQL si une URL est fournie. Résumé final ligne par ligne, code de
sortie 0 seulement si tout est vert.

### Documentation des décisions

`decisions/` : ADR-002 (persistance locale), ADR-003 (TUI), ADR-004 (Chromat),
ADR-005 (Ratatui/formulaires), **ADR-006** (Rustorm sans base), **ADR-007**
(sécurité locale : PBKDF2, TOTP, audit chaîné), **ADR-008** (PDF/e-mail/CSV/
e-facture sans dépendance). Traçabilité du CDC : `CONFORMITE-CDC.md`.

## 🚀 Installer, compiler et lancer en production

### 0. Une seule commande (recommandé)

```bash
git clone https://github.com/jojo8356/my_ERP.git && cd my_ERP
./scripts/installer-prod.sh                     # clone réutilisé, compile, installe
./scripts/installer-prod.sh --avec-postgres     # + connecteur libpq
./scripts/installer-prod.sh --aide              # toutes les options
```

Sur une machine vierge (une fois la branche fusionnée dans `main`, le script est
aussi servi tel quel) :

```bash
curl -fsSL https://raw.githubusercontent.com/jojo8356/my_ERP/main/scripts/installer-prod.sh \
  | bash -s -- --branche main
```

Le script vérifie les prérequis (git, Rust stable, accès au dépôt, `libpq` si
demandé), clone, compile en `--release`, installe le binaire, crée le dossier de
données et contrôle que l'application démarre. Il ne crée **aucune** donnée
métier : le premier lancement reste explicite (étape 5).

### 1. Prérequis

| Besoin | Détail |
|---|---|
| Rust stable | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh -s -- -y --profile minimal` |
| git + réseau | `chromat` et `rustorm` sont des dépendances **git** (`github.com/jojo8356`) |
| `libpq-dev` | uniquement pour `--features postgres` (`apt install libpq-dev`) |
| Un vrai terminal | la TUI passe le terminal en mode brut (pas de `nohup`) |

### 2. Cloner

```bash
git clone https://github.com/jojo8356/my_ERP.git
cd my_ERP
git checkout main        # une fois la PR #3 fusionnée
# sinon, pour construire la version livrée avec prospects, notes et sélecteurs :
git fetch origin arena/768a6cd6-my-erp
git checkout arena/768a6cd6-my-erp
```

### 3. Compiler (release)

```bash
. "$HOME/.cargo/env"                 # si rustup vient d'être installé
cargo build --release                # mode par défaut : fichiers locaux, aucune base
# variante PostgreSQL (état dans la base, via Rustorm) :
sudo apt install -y libpq-dev && cargo build --release --features postgres
./target/release/my-erp --version
```

Le binaire est **autonome** : `target/release/my-erp` (aucun runtime, aucune
dépendance à installer sur le serveur, hors `libpq` si la feature est utilisée).

### 4. Installer et préparer les données

```bash
sudo install -m 755 target/release/my-erp /usr/local/bin/my-erp
sudo mkdir -p /var/lib/my-erp && sudo chown "$USER":"$(id -gn)" /var/lib/my-erp
export ERP_DONNEES_DIR=/var/lib/my-erp       # à mettre dans ~/.bashrc ou le service
my-erp --version
```

`ERP_DONNEES_DIR` (ou `--donnees <dossier>`) désigne l'état ; sans lui, le
binaire travaille dans `./donnees`. **Un seul processus écrivant à la fois** sur
un même dossier de fichiers : un ERP de service, pas un cluster.

### 5. Premier lancement (identité et compte administrateur)

```bash
my-erp /societe modifier --nom "Ma société" --siren 123456789 \
        --adresse "1 rue de la Paix" --cp 06000 --ville Nice --forme micro-entreprise
my-erp /utilisateurs nouveau admin --role admin --mot-de-passe "MotDePasse-2026!" \
        --email moi@exemple.fr
my-erp /utilisateurs 2fa admin         # SEC-02 : double authentification obligatoire
my-erp /utilisateurs code admin        # ou un code à usage unique par e-mail (SEC-03)
```

### 6. Usage quotidien en production

```bash
my-erp tui                             # TUI : e = société, u = session, c = statut, o = note
my-erp /auth connexion admin --mot-de-passe … --totp 123456
my-erp /factures --impayees --retard   # scriptable : code de sortie 0/1
my-erp /compta fec > fec.csv           # FEC DGFiP
my-erp /admin sante                    # fichiers, lignes, sauvegarde, chaîne d'audit
my-erp /admin sauvegarde               # copie datée dans donnees/sauvegardes/
my-erp /admin audit --verifier         # détecte toute altération du journal
```

Sauvegarde de production : `/admin sauvegarde` (mode fichiers) ou `pg_dump`
(mode PostgreSQL, à cadrer par la politique de rétention) — la copie du dossier
`donnees/` est équivalente et suffit pour un redémarrage à l'identique.

### 7. PostgreSQL en production — ce qui marche aujourd'hui

```bash
sudo -u postgres createuser erp
sudo -u postgres createdb -O erp erp
# le connecteur local de Rustorm ne transmet pas de mot de passe (SEC-07) :
# pour 127.0.0.1/localhost, autorisez le compte local dans pg_hba.conf
#   local   erp   erp                     peer
#   host    erp   erp   127.0.0.1/32     trust
export ERP_DATABASE_URL=postgres://erp@127.0.0.1:5432/erp
my-erp /admin bdd          # migration par l'ORM (CREATE TABLE + index), puis état
my-erp /clients            # mêmes commandes, état lu/écrit dans la base
```

Une base **distante** passe par le mode sécurisé de Rustorm (`sslmode=verify-full`,
certificat de l'autorité attendu) : `postgres://erp:motdepasse@db.exemple.fr:5432/erp`.

### 8. Service au quotidien (systemd, facultatif)

```ini
# /etc/systemd/system/my-erp@.service  →  systemctl start my-erp@admin
[Unit]
Description=ERP CLI (session %i)
[Service]
Type=oneshot
RemainAfterExit=no
User=erp
Environment=ERP_DONNEES_DIR=/var/lib/my-erp
Environment=NO_COLOR=1
ExecStart=/usr/local/bin/my-erp /admin sante
```

Les tâches récurrentes (relances, sauvegardes) sont des **commandes** : un
`cron` les appelle, par exemple `0 7 * * 1 my-erp /admin sauvegarde`.
