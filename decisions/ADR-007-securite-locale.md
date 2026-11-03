# ADR-007 — Sécurité locale : rôles, mots de passe, 2FA, audit chaîné

Date : 02/11/2026 · Statut : acté (SEC-01 à SEC-06 du CDC, sans base de données)

## Contexte

Le CDC exige des comptes nominatifs, la séparation des rôles, la double
authentification pour les administrateurs, des mots de passe jamais stockés en
clair et un journal d'audit infalsifiable. La V1 tourne sur un poste unique, sans
serveur : il n'y a ni annuaire, ni TLS, ni service d'authentification à qui
déléguer. Les primitives doivent donc être écrites **dans le dépôt**, sans
dépendance cryptographique externe (l'environnement de développement bloque
crates.io ; c'est aussi ce qui garantit un binaire unique — ENF-SIMPL-01).

## Décision

Tout est implémenté dans `src/securite.rs`, sur la seule base de
`rustorm::checksum::sha256` (déjà présent pour les empreintes du schéma) :

| Besoin | Choix | Détail |
|---|---|---|
| Rôle | `Role::Admin` / `Role::Utilisateur` | persisté en texte (`admin`), traduit par `depuis_code` |
| Mot de passe | PBKDF2-HMAC-SHA256, 120 000 itérations | sel de 16 octets par compte, format `pbkdf2-sha256$120000$<sel hex>$<clé hex>` |
| Comparaison | `egalite_constante` | pas de sortie anticipée (attaque temporelle) |
| Politique | ≥ 12 caractères, 3 familles sur 4, pas de mot courant | message en français expliquant quoi corriger |
| 2FA | TOTP RFC 6238 (SHA-1, 6 chiffres, pas 30 s, ±1 pas) | secret base32 de 20 octets, URI `otpauth://` pour l'application d'authentification |
| Codes de secours | 8 codes `abcd-ef12` à usage unique | seule l'empreinte SHA-256 est stockée, le code est consommé à la connexion |
| Audit | chaîne de hachage | chaque entrée contient l'empreinte de la précédente ; `verifier_chaine_audit()` rejoue la chaîne |

### Règles de comportement

- **Mode mono-utilisateur** : tant qu'aucun compte n'existe, l'ERP fonctionne
  sans authentification (usage quotidien d'un indépendant, aucune barrière
  inutile) ; toute action est journalisée avec l'utilisateur `local`.
- **Dès qu'un compte existe**, les actions sensibles (validation d'une facture,
  restauration de sauvegarde, gestion des comptes) exigent le rôle admin
  (`Store::exiger_admin`). Le premier compte créé doit être administrateur,
  sinon plus personne ne pourrait administrer l'outil.
- **2FA obligatoire pour les administrateurs** : la connexion d'un compte admin
  exige un code TOTP ou un code de secours. Un échec est journalisé
  (`auth.echec`), y compris quand le mot de passe était bon.
- **Session locale** : le nom du compte connecté est stocké dans
  `donnees/meta.tsv` (déjà restreint au compte système) ; `/auth deconnexion`
  revient au mode `local`. Il n'y a pas de jeton réseau : il n'y a pas de
  réseau.
- **RGPD** : l'export (`/clients <nom> rgpd exporter`) et l'effacement
  (`… rgpd effacer`) anonymisent la fiche client tout en conservant les pièces
  comptables 10 ans (obligation légale), et les deux opérations sont
  journalisées.

## Conséquences

- Aucune dépendance ajoutée, aucune bibliothèque cryptographique tierce à
  auditer ; les primitives sont testées (vecteurs RFC 6238, aller-retour de
  hachage, détection d'une entrée d'audit modifiée).
- L'aléa provient de `SystemTime` + identifiant de processus + adresse d'une
  variable locale, haché par SHA-256. **Ce n'est pas un CSPRNG** ; c'est
  suffisant pour des sels, des secrets TOTP et des codes de secours sur un poste
  unique, et c'est documenté comme tel. Le jour où l'ERP devient multi-postes
  (jalon J5 avec PostgreSQL), la source d'aléa doit être remplacée par
  `getrandom`.
- Pas de TLS, pas de SMTP authentifié : les e-mails sont **écrits** dans
  `donnees/courriels/*.eml` (voir ADR-008). Aucun secret de messagerie n'est
  stocké.
- SEC-06 (aucune donnée bancaire complète) est tenu par construction : le
  rapprochement n'importe qu'un relevé (date, libellé, montant, référence) et
  ne stocke aucun identifiant bancaire.

## Alternatives écartées

- **`argon2`/`ring`/`rustls`** : dépendances natives ou lourdes, impossibles à
  télécharger dans l'environnement de développement et inutiles pour un poste
  unique — ADR à réviser au J5.
- **PAM / comptes système** : lie l'ERP aux comptes de la machine, exclut
  Windows/macOS (ENF-COMPAT-01) et empêche la séparation des rôles métier.
- **Pas d'authentification du tout** : viole SEC-01 à SEC-03 dès que l'outil
  sert à plusieurs personnes (l'ERP est prévu pour ×2 utilisateurs).
