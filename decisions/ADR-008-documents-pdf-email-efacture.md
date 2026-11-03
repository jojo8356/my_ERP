# ADR-008 — PDF, e-mail, CSV et e-facture écrits dans le dépôt

Date : 02/11/2026 · Statut : acté (EF-FAC-05, EF-FAC-07, INT-01, INT-03, ENF-COMPAT-01)

## Contexte

Une facture doit sortir en PDF conforme (§11.1), être envoyable par e-mail en
une commande (EF-FAC-05), et l'ERP doit produire des exports CSV/FEC lisibles
par un expert-comptable (EF-CPT-03, INT-03) ainsi qu'une e-facture structurée
(EF-FAC-07). Les bibliothèques habituelles (`printpdf`, `lettre`, `quick-xml`,
`csv`, `reqwest`…) sont autant de dépendances à télécharger, à auditer et à
maintenir ; l'environnement de développement de la V1 n'y a de toute façon pas
accès (registry crates.io bloqué), et ENF-SIMPL-01 demande un binaire unique
sans prérequis.

## Décision

Chaque format est produit par un module court, testé, sans dépendance :

| Module | Ce qu'il produit | Choix technique |
|---|---|---|
| `src/pdf.rs` | factures, avoirs | écriture PDF 1.4 directe : objets, `xref` calculée, polices Helvetica/Helvetica-Bold base 14, texte en **WinAnsiEncoding** (les accents sont des octets Latin-1), pagination automatique, montants alignés |
| `src/messagerie.rs` | e-mails (facture, devis, relances 1-3, code de connexion) | message **RFC 5322** complet : en-têtes `From/To/Subject/Date/Message-ID`, sujet non-ASCII encodé RFC 2047, corps `multipart/mixed` avec PDF en base64 (lignes de 76 caractères), écrit dans `donnees/courriels/*.eml` |
| `src/csv.rs` | exports de toutes les listes, imports (clients, articles, ventes, relevés) | RFC 4180 : séparateur `;` ou `,` détecté, guillemets doublés, en-têtes normalisés (accents/casse ignorés), montants `1 234,56` acceptés |
| `src/efacture.rs` | e-facture UBL 2.1 / EN 16931 | XML généré à la main, `CustomizationID urn:cen.eu:en16931:2017`, types 380/381/384, `TaxSubtotal` par taux, `VATEX-EU-132` en franchise, échappement XML systématique |
| `src/compta.rs` | FEC DGFiP, journaux, balance | 18 colonnes `|`, dates `AAAAMMJJ`, montants `1234,56`, libellés sans `|` |

## Conséquences

- **Aucune surface d'attaque réseau** : rien n'est envoyé. L'intégration SMTP
  (INT-01) se branchera sur `messagerie::enveloppe()` — le message complet est
  déjà produit, seule la transmission manque ; c'est le seul endroit à écrire au
  jalon J6.
- **Formats vérifiables en test** : les tests unitaires inspectent les octets
  (en-tête `%PDF-1.4`, `xref`, présence des mentions obligatoires en WinAnsi,
  en-têtes MIME, `TaxAmount`, en-tête FEC à 18 colonnes).
- **Limites assumées** :
  - PDF sans images ni polices embarquées (Helvetica base 14) : conforme pour
    une facture de services, suffisant mais sobre ;
  - UBL sans signature électronique (INT-06 est classé C dans le CDC) et sans
    transmission via plateforme agréée (dépend du prestataire, jalon J6) ;
  - réception d'e-factures fournisseurs (INT-02) : non implémentée en v1.1, la
    trace est dans `CONFORMITE-CDC.md`.
- **Fidélité au CDC** : les mentions §11.1 (identité, SIREN, adresse, date,
  échéance, conditions, pénalités, indemnité forfaitaire 40 €, franchise 293 B)
  sont testées une par une ; c'est `efacture`/`pdf` qui les portent, le métier
  (`domain::controle_mentions`) décidant de la validation.

## Alternatives écartées

- `printpdf` + `lettre` + `csv` + `quick-xml` : quatre dépendances lourdes pour
  quatre formats simples, non téléchargeables dans l'environnement de V1.
- Générer du HTML puis « imprimer en PDF » : exige un navigateur, donc une GUI
  — interdit par le CDC.
- Laisser l'utilisateur faire ses PDF ailleurs : viole EF-FAC-05 (« en un
  commande ») et la cohérence des mentions.
