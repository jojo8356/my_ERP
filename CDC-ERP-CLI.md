# CAHIER DES CHARGES — ERP CLI en Rust
### « Aller plus vite qu'OpenConcerto et Dolibarr, sans aucun menu »

| | |
|---|---|
| **Projet** | my_ERP — ERP personnel en ligne de commande (CLI), Rust + PostgreSQL |
| **Version** | CDC v1.1 — 30 octobre 2026 — **réponses Q1–Q6 intégrées (§21), EN ATTENTE DE VOTRE GO J0** |
| **Commanditaire / seul utilisateur v1** | Vous (TPE, secteur services) — validation exigée à chaque grande étape |
| **Sources des choix** | `decisions/mes-choix.json` (vos réponses du 30/10/2026) |
| **Méthode** | CDC construit comme dans les grandes entreprises : modèle **Volere** (exigences fonctionnelles / non-fonctionnelles / contraintes), qualité **ISO/IEC 25010:2023**, priorisation **MoSCoW**, critères d'acceptation **Given/When/Then**, traçabilité exigence → test (voir §2) |

> ⚖️ **Règle d'or du document** : vos choix explicites priment. Vous avez imposé **Rust partout et zéro GUI** :
> la recommandation automatique initiale (React/Python) est donc **écartée** et cet écart est acté ici (§5.4).

---

## Table des matières
1. Contexte, problème et objectifs mesurables — 2. Méthode & gouvernance — 3. Parties prenantes
4. Périmètre (dans / hors) — 5. Solution cible & architecture — 6. Parcours de navigation CLI (votre exigence n°1)
7. Exigences fonctionnelles par module — 8. Modèle de données — 9. Exigences non fonctionnelles (ISO 25010)
10. Sécurité & RGPD — 11. Conformité métier : services informatiques & vente de vêtements (Vinted) — sources officielles
12. Intégrations — 13. Migration / reprise des données — 14. Déploiement, DevOps, exploitation
15. Stratégie de test & recette — 16. Planning par jalons validés — 17. Budget & TCO — 18. Risques
19. Critères d'acceptation du projet — 20. Points ouverts à trancher — Annexes : glossaire, sources, traçabilité

---

## 1. Contexte, problème et objectifs

### 1.1 Contexte
TPE de services (< 10 personnes), aujourd'hui sur **OpenConcerto et Dolibarr**. Constat (votre formulation) :
*« pleins de menus, c'est dur d'aller vite »*. Vous voulez **d'abord un outil pour vous**, en CLI, durable,
remplaçant l'ERP existant, utilisable à terme par « tous les employés » (aujourd'hui : 1 utilisateur, ×2 à 3 ans).

Vos deux activités que l'ERP doit couvrir (déduites de votre demande « règles spécifiques ») :
1. **Prestations de services informatiques** (facturation de services, TVA sur les encaissements/débits, devis).
2. **Vente de vêtements, notamment via Vinted** (achat-revente et/ou occasion — régime à trancher, §20).

### 1.2 Objectifs mesurables (les grandes entreprises n'acceptent pas un CDC sans KPI)
| # | Objectif | Mesure cible v1 | Vérifié par |
|---|---|---|---|
| OBJ-1 | Émettre une facture simple | ≤ 60 s, ≤ 3 commandes depuis l'accueil du CLI | Test chronométré en recette (§15) |
| OBJ-2 | Retrouver un client / une facture | ≤ 5 s via chemin mémorisé ou recherche | Test chronométré |
| OBJ-3 | Zéro navigation dans des menus | 100 % des actions v1 accessibles par chemin direct + raccourci | Revue de la table des commandes (§6) |
| OBJ-4 | Conformité facturation | 100 % des mentions obligatoires présentes (§11.1), numérotation sans trou | Checklist de recette légale |
| OBJ-5 | Aucune perte de données | Restauration d'une sauvegarde quotidienne réussie en ≤ 30 min | Test de restauration avant go-live |
| OBJ-6 | Traçabilité conformité | 100 % des écritures sensibles journalisées (qui/quoi/quand) | Revue du journal d'audit |

## 2. Méthode & gouvernance (façon grande entreprise, adaptée à 1 personne + IA)

- **Exigences numérotées et traçables** : `EF-xxx` (fonctionnel), `ENF-xxx` (non-fonctionnel), `SEC-xxx`, `RGPD-xxx`.
  Chaque exigence « Must » aura au moins un test d'acceptation (§15). C'est la pratique Volere / ISO-IEEE 29148.
- **Priorisation MoSCoW** : **M**ust (v1 bloquante), **S**hould (v1 si le délai tient), **C**ould (v1.x), **W**on't (explicitement exclu, §4.2).
- **Critères d'acceptation** rédigés en *Given/When/Then* pour les fonctions critiques (facturation en tête).
- **Qualité logicielle** cadrée par **ISO/IEC 25010:2023** (9 caractéristiques) pour les ENF du §9.
- **Gouvernance** : vous validez **chaque jalon** (§16) avant que le code du jalon suivant démarre — c'est votre choix « je veux valider chaque grande étape ». Un jalon = démonstration CLI + tests verts + mise à jour de ce CDC si écart.
- **Journal de décisions (ADR simplifié)** : tout écart à ce CDC est écrit dans `decisions/` avant d'être codé.

## 3. Parties prenantes et rôles

| Acteur | Rôle dans le projet | Droits dans l'outil |
|---|---|---|
| Vous (propriétaire TPE) | Commanditaire, valideur de chaque jalon, utilisateur principal | **Admin** |
| Futurs employés (×2 à 3 ans) | Utilisateurs | **Utilisateur** (périmètre restreint, §10) |
| Expert-comptable | Destinataire des exports (FEC, journaux) | Aucun accès direct v1 (exports fichiers) |
| Clients (services info / acheteurs) | Reçoivent devis/factures (e-mail/PDF) | Aucun accès v1 |
| Plateforme agréée (facturation élec.) | Achemine les e-factures | Connecteur (§12), jamais d'accès à la base |
| Vinted / Stripe / banque | Sources de transactions | Import / connecteurs (§12), identifiants en coffre (§10) |

## 4. Périmètre

### 4.1 Dans le périmètre v1 (Must/Should)
- **Socle** : CLI Rust rapide (navigation par chemins + raccourcis), multi-sociétés, multi-devises, PostgreSQL, auth + 2FA, rôles simples, journal d'audit, sauvegardes quotidiennes, import/export Excel-CSV.
- **Module A — CRM clients** (v1) : fiches clients/prospects, contacts, historique (devis, factures, ventes).
- **Module B — Facturation & devis** (v1, **module critique n°1**) : devis → facture, mentions légales FR, TVA, acomptes, avoirs, échéances, relances, PDF, envoi e-mail, préparation facturation électronique.
- **Module C — Comptabilité / Finance** (v1) : écritures issues des factures, journaux, TVA collectée/déductible, rapprochement bancaire simple, export expert-comptable (**FEC**).
- **Module D — E-commerce / Vinted** (v1) : catalogue vêtements (taille, marque, composition, état, prix d'achat), suivi des ventes Vinted (import CSV/manuel), marge par article, stock simple lié.
- **Préparation** (architecture, pas d'écran) : API REST du socle pour la future application mobile ; multi-langue **non** (français uniquement).

### 4.2 Hors périmètre v1 (Won't — pour tenir « quelques semaines »)
GUI / interface web ❌ (votre contrainte absolue) · application mobile (plus tard) · paie/RH · production/atelier ·
multi-dépôts avancé · signature électronique intégrée (Could v1.x) · connecteur Vinted « automatique » par scraping
(interdit par les CGU des plateformes — uniquement API officielle si elle existe ou import CSV, §11.3) ·
devenir soi-même plateforme agréée de facturation électronique ❌ (on se connecte à une plateforme agréée existante).

### 4.3 Versions suivantes (vision)
v1.x : signature électronique, SMS de relance, rapprochement Stripe automatique. v2 : application mobile (via API REST), module Projets/Tâches complet (souhaité mais hors v1), Ventes/Commandes avancé.

## 5. Solution cible & architecture

### 5.1 Stack arrêtée (vos choix + arbitrages)
| Couche | Choix | Justification |
|---|---|---|
| Langage unique | **Rust** (CLI + cœur + futur serveur) | Votre contrainte imposée : sécurité mémoire, binaire unique, pas de fuites |
| Interface | **CLI Rust** : commandes à chemins type URL, mode interactif (REPL), complétion shell, raccourcis clavier | Votre exigence « aller vite », zéro GUI |
| Architecture | **Monolithe modulaire** : `erp-core` (métier) + `erp-cli` (v1) + `erp-server` REST (préparé, activé pour le mobile) | Simple à maintenir (votre priorité 5/5), découpable plus tard |
| Base de données | **PostgreSQL** (on-premise, serveur interne) | Votre choix ; n°1 du marché, gratuit, robuste |
| API | **REST** (JSON), versionnée `/api/v1` | Votre choix ; servira au mobile et aux exports |
| PDF factures | Génération PDF côté Rust (template FR conforme) | Mentions légales §11.1 |
| Hébergement | **Serveur interne** (on-premise), Linux | Votre choix ; aucun coût cloud récurrent (budget minimal) |
| DevOps | Docker (PostgreSQL + service), **CI/CD** (tests auto à chaque commit), staging + production | Votre choix « standard » |

### 5.2 Découpage modulaire (un module = un dossier du core, activable/désactivable par société)
`core` (sociétés, utilisateurs, audit, devises) · `crm` · `facturation` · `comptabilite` · `ecommerce` (vinted) · (`projets`, `commandes` : stubs v2).

### 5.3 Principes d'architecture (non négociables)
Séparation stricte CLI ↔ métier ↔ données (le futur serveur REST réutilise 100 % du métier) · aucune règle de TVA/compta codée « en dur » dans le CLI (tout est dans `erp-core`, testé unitairement) · montants en **entiers de centimes** (jamais de flottants) · devises ISO 4217 avec taux datés · toutes les dates métier en fuseau `Europe/Paris` · migrations de base versionnées et réversibles.

### 5.4 Écart assumé par rapport à la recommandation automatique
L'artefact recommandait React/Python. **Vous avez imposé Rust + CLI** : cet écart est validé par le présent CDC.
Conséquence assumée : pas de GUI (ni en v1, ni prévue), l'« esthétique » (votre priorité absolue) se traduit en
**ergonomie terminal** : sorties alignées, couleurs sobres, aide contextuelle, zéro écran inutile (§6, §9-ENF-ERG).

## 6. Parcours de navigation CLI — votre exigence n°1 « aller vite »

### 6.1 Navigation par chemins (comme des URL, mémorisables)
```
erp /                                  # accueil : tableau de bord texte (impayés, ventes du mois)
erp /clients                           # liste clients        (alias : erp c)
erp /clients/dupont                    # fiche client (recherche floue sur le nom)
erp /clients/dupont/factures           # ses factures
erp /factures                          # liste factures       (alias : erp f)
erp /factures/2026/0042                # facture FA-2026-0042 (chemin direct si vous connaissez le numéro)
erp /factures/nouvelle --client dupont # création guidée minimale (3 questions max)
erp /devis /compta /vinted /stock      # modules v1
erp go /clients/dupont/factures        # en mode interactif : navigation sans relancer le binaire
```
Règles : tout chemin fonctionne en **commande unique** (scriptable) et en **mode interactif** (`erp` seul ouvre un prompt) ; segments insensibles à la casse ; recherche floue (fautes tolérées) ; `..` remonte d'un niveau ; `erp aide <chemin>` (ou `?`) affiche l'aide **du contexte**.

### 6.2 Raccourcis (mode interactif) — « le plus de raccourcis clavier intelligents »
| Raccourci | Action | | Raccourci | Action |
|---|---|---|---|
| `Ctrl+R` | Recherche dans l'historique | | `g f` / `g c` / `g v` | Aller à Factures / Clients / Vinted |
| `Tab` | Complétion (chemins, clients, n° de facture) | | `n` | Nouveau (contexte : facture si on est dans /factures…) |
| `Ctrl+K` | Palette de commandes (recherche globale) | | `e` / `Entrée` | Ouvrir l'élément sélectionné |
| `/` | Recherche globale immédiate | | `p` | Marquer payé (sur une facture) |
| `q` / `Échap` | Retour / quitter le contexte | | `x` | Exporter la vue courante (CSV/PDF) |
| `?` | Aide contextuelle | | `Ctrl+S` | Sauvegarder un brouillon en cours |

Exigences : raccourcis **affichés en permanence** en pied d'écran interactif (1 ligne), personnalisables dans `~/.config/erp/raccourcis.toml`, aucun raccourci ne déclenche d'action destructive sans confirmation (`suppr` exige de retaper la référence).

### 6.3 Sorties « belles et agréables » (votre priorité absolue, traduite CLI)
Tableaux alignés à colonnes fixes, montants alignés à droite avec devise, statuts colorés sobres
(texte **gras** = impayé en retard, vert = payé — toujours doublés d'un symbole `✓/!` pour ne pas dépendre que de la couleur),
pagers intelligents (jamais 500 lignes d'un coup), `NO_COLOR` respecté, sortie `--json` sur toute commande (scriptabilité).

## 7. Exigences fonctionnelles (extraits normatifs — Must = v1)

### Socle
- **EF-SOC-01 (M)** Multi-sociétés : bascule `erp /societes/<nom>`, données cloisonnées par société, devise par défaut par société.
- **EF-SOC-02 (M)** Multi-devises : montants saisis en devise du document, conversion au taux daté, totaux société en devise de référence.
- **EF-SOC-03 (M)** Recherche globale (`/`) sur clients, factures, articles en < 1 s sur le volume cible (< 10 000 lignes/module).

### Module B — Facturation & devis (critique)
- **EF-FAC-01 (M)** Devis : création, statuts (brouillon/envoyé/accepté/refusé/expiré), transformation en facture en 1 commande.
- **EF-FAC-02 (M)** Facture : **numérotation chronologique continue sans trou** par société et par exercice ; impossible de modifier/supprimer une facture validée (avoir obligatoire pour corriger).
  *Acceptation — Given* une facture validée, *When* je tente de l'éditer, *Then* le CLI refuse et propose « créer un avoir ».
- **EF-FAC-03 (M)** Lignes : désignation, quantité (heures/jours/forfait pour les services ; article pour les vêtements), prix unitaire HT, remise, **taux de TVA par ligne**, totaux HT/TVA/TTC calculés automatiquement.
- **EF-FAC-04 (M)** Acomptes et solde, échéance, conditions de paiement, pénalités de retard et indemnité forfaitaire 40 € (B2B) paramétrables.
- **EF-FAC-05 (M)** PDF conforme (§11.1) + envoi e-mail en 1 commande (`erp /factures/2026/0042 envoyer`).
- **EF-FAC-06 (M)** Suivi des paiements : impayés, retards, relances (modèles e-mail), `p` = marquer payé avec date et moyen.
- **EF-FAC-07 (S)** Préparation e-facture : export structuré (Factur-X/UBL) et envoi via la plateforme agréée choisie (§11.1, §12).
- **EF-FAC-08 (S)** Factures récurrentes (abonnements de maintenance informatique).

### Module A — CRM
- **EF-CRM-01 (M)** Fiche client : identité complète (SIREN, TVA intracom. si pro), type (particulier/pro/asso), adresses, contacts, notes datées.
- **EF-CRM-02 (M)** Historique unifié : devis, factures, paiements, ventes liées, depuis `/clients/<nom>`.
- **EF-CRM-03 (S)** Étiquettes et recherche (ex. `clients --tag maintenance`), export CSV/Excel.

### Module C — Comptabilité / Finance
- **EF-CPT-01 (M)** Écritures générées automatiquement depuis factures/paiements (plan comptable FR paramétrable), journaux (ventes, banque, achats).
- **EF-CPT-02 (M)** TVA : TVA collectée/déductible par taux et par période, récapitulatif prêt pour la déclaration (services : encaissements ou débits selon régime §11.2).
- **EF-CPT-03 (M)** Export **FEC** conforme (format DGFiP) + exports journaux pour l'expert-comptable.
- **EF-CPT-04 (S)** Rapprochement bancaire : import relevé (CSV/OFX), lettrage semi-automatique ; **aucune donnée bancaire complète stockée** (tokens/4 derniers chiffres uniquement, SEC-06).
- **EF-CPT-05 (S)** Tableau de bord finance : CA par mois/activité (services vs vêtements), encours clients, trésorerie simple.

### Module D — E-commerce / Vinted
- **EF-VIN-01 (M)** Article vêtement : photos (liens fichiers), marque, taille, état, **composition textile**, prix et date d'achat, source (achat-revente ou bien personnel), statut (en stock/en ligne/vendu).
- **EF-VIN-02 (M)** Vente : prix de vente, frais de plateforme/port, **marge nette calculée**, distinction du régime fiscal par article (§11.3 : personnel exonéré vs achat-revente BIC).
- **EF-VIN-03 (M)** Import CSV des ventes (export Vinted / saisie assistée), aucune automatisation contraire aux CGU.
- **EF-VIN-04 (S)** Statistiques : rotation du stock, marge par marque/taille, cumul annuel avec **alerte aux seuils DAC7** (30 ventes / 2 000 €, §11.3).
- **EF-VIN-05 (C)** Fiche produit exportable (description + composition + taille) prête à coller dans l'annonce.

## 8. Modèle de données (conceptuel, à détailler au jalon J1)

Entités : `societe` — `utilisateur(role)` — `client(contact)` — `devis(ligne)` — `facture(ligne, paiement, avoir)` —
`ecriture(journal)` — `article_vetement(vente_vinted)` — `produit_service` — `taux_tva` — `taux_change` — `audit_log` — `sauvegarde`.
Règles : suppression logique (jamais physique) pour tout document comptable ; horodatage `créé_le/modifié_le/par` partout ;
index sur numéros de documents et noms clients ; clés étrangères strictes ; contraintes d'unicité sur les numéros de facture.

## 9. Exigences non fonctionnelles (ISO/IEC 25010:2023, chiffrées — vos pondérations entre parenthèses)

| ID | Caractéristique | Exigence mesurable |
|---|---|---|
| ENF-PERF-01 | Performance (1/5 : modeste) | Toute commande courante < 500 ms en local sur le volume cible ; démarrage du CLI < 300 ms |
| ENF-SIMPL-01 | Maintenabilité (5/5 : **votre max**) | Binaire unique + PostgreSQL seuls prérequis ; installation = 1 script ; code métier couvert ≥ 80 % par les tests (facturation/compta : 100 % des règles de calcul) |
| ENF-ERG-01 | Interaction (4/5) | OBJ-1/2/3 du §1.2 ; aide contextuelle partout ; messages d'erreur en français, actionnables (« quoi faire » inclus) |
| ENF-FIA-01 | Fiabilité | Aucune écriture comptable à moitié enregistrée (transactions) ; reprise propre après coupure |
| ENF-SCAL-01 | Flexibilité (3/5) | ×2 utilisateurs et ×10 lignes sans changement d'architecture ; ajout d'un module sans toucher aux modules existants |
| ENF-COMPAT-01 | Compatibilité | Linux (prod), poste de travail Linux/macOS/Windows (CLI) ; exports CSV/XLSX/PDF/FEC ouverts |
| ENF-PORT-01 | Portabilité | Données intégralement exportables (dump PostgreSQL + CSV) — aucun verrouillage |
| ENF-COUT-01 | Coût (0/5 noté, mais budget « minimal ») | 100 % open source, 0 licence, hébergement = serveur déjà possédé (§17) |

## 10. Sécurité & RGPD

- **SEC-01 (M)** Mots de passe hachés **Argon2id**, politique de robustesse, aucun secret en clair dans la base ni les journaux.
- **SEC-02 (M)** **2FA TOTP** obligatoire pour les comptes admin ; codes de récupération à usage unique.
- **SEC-03 (M)** Connexion par **code e-mail / lien magique** en alternative (votre choix) : lien à durée ≤ 15 min, usage unique.
- **SEC-04 (M)** Rôles simples : **Admin** (tout) / **Utilisateur** (modules autorisés, pas d'accès paie/finance sensibles — il n'y a pas de paie en v1).
- **SEC-05 (M)** **Journal d'audit** infalsifiable en append-only : connexion, création/validation de facture, paiement, export, suppression logique (qui, quoi, quand, avant/après pour les champs sensibles).
- **SEC-06 (M)** Données bancaires : **jamais** de numéro de carte/IBAN complet stocké ; uniquement jetons d'API (Stripe) et 4 derniers chiffres ; clés API dans un coffre (variables d'environnement / fichier `0600`, jamais dans Git).
- **SEC-07 (M)** Chiffrement : disque/base chiffrés au repos sur le serveur interne, TLS dès que `erp-server` est activé, Rust = pas d'erreurs mémoire (unsafe interdit hors justification écrite).
- **RGPD-01 (M)** Registre des traitements tenu dans `decisions/` (finalités, durées) — exigence CNIL de durée définie par finalité.
- **RGPD-02 (M)** Durées par défaut proposées (à valider §20) : prospects 3 ans après dernier contact ; clients = durée de la relation puis archivage comptable **10 ans** (pièces justificatives, Code de commerce) ; accès/rectification/effacement/export en 1 commande (`erp /clients/<nom> rgpd exporter|effacer`) avec réponse sous 1 mois.
- **RGPD-03 (M)** Minimisation : aucune donnée client non nécessaire à la facturation/vente n'est demandée par le CLI.

## 11. Conformité métier — lois & normes (sources officielles, vérifiées le 30/10/2026)

### 11.1 Facturation (services informatiques et ventes) — le cœur critique
- **Mentions obligatoires** sur chaque facture (CGI ann. II art. 242 nonies A ; C. com. L441-9) : identité complète
  émetteur (forme juridique, capital, SIREN/SIRET, TVA intracom.) et client, n° unique chronologique, dates d'émission
  et de prestation, désignation précise (quantité, prix unitaire HT), TVA par taux ou mention
  **« TVA non applicable, art. 293 B du CGI »** si franchise en base, échéance, pénalités de retard, indemnité forfaitaire 40 €.
  → implémentées comme un **modèle contrôlé automatiquement** avant validation (EF-FAC-02/05).
- **Facturation électronique (réforme en cours, dates officielles economie.gouv.fr)** : depuis le **1er septembre 2026,
  toute entreprise doit pouvoir RECEVOIR** des e-factures via une **plateforme agréée** ; les TPE/PME devront
  **ÉMETTRE** au format électronique à partir du **1er septembre 2027** (+ e-reporting). Quatre nouvelles mentions
  s'ajoutent (SIREN client, nature de l'opération…). **Exigence CDC** : le module Facturation doit être prêt pour
  l'émission via plateforme agréée **avant l'été 2027** (jalon J6) ; dès la v1, choisir la plateforme et pouvoir recevoir.
  Un simple PDF envoyé par e-mail ne vaudra plus « facture électronique » au sens de la réforme.
- **Comptabilité** : écritures chronologiques, export **FEC** au format normé DGFiP en cas de contrôle,
  conservation des pièces comptables **10 ans**, numérotation sans rupture (EF-FAC-02, EF-CPT-03).

### 11.2 Services informatiques (prestations)
- Facture **obligatoire entre professionnels** ; pour un particulier : facture/note dès 25 € TTC ou sur demande.
- TVA prestations de services : **exigible à l'encaissement** par défaut (option possible pour les débits — à trancher §20) ;
  taux normal 20 % sauf cas particuliers ; prestations UE B2B : autoliquidation avec TVA intracom. du client (EF-FAC-03 doit gérer ces cas).
- Devis : mentions et durée de validité ; le devis accepté vaut engagement — d'où EF-FAC-01 (traçabilité des statuts).

### 11.3 Vente de vêtements & Vinted — **le point juridique le plus sensible**
- **Deux régimes à ne jamais confondre dans l'outil (EF-VIN-02)** :
  (a) **Biens personnels d'occasion** vendus sans but lucratif → en principe **non imposables, rien à déclarer**, quel que soit le montant ;
  (b) **Achat (ou fabrication) pour revendre** → **activité commerciale (BIC) dès le premier euro** : immatriculation
  (micro-entreprise le plus souvent), déclaration du CA, cotisations. Il n'existe **pas de seuil** qui « autorise » l'achat-revente non déclaré.
- **DAC7** : Vinted transmet au fisc les vendeurs au-delà de **30 transactions ou 2 000 €/an** — c'est un seuil de
  **signalement, pas d'imposition** ; l'ERP alerte à l'approche des seuils (EF-VIN-04) et conserve les justificatifs d'achat.
- **Si vente en tant que professionnel** (site ou plateformes en mode pro) : information précontractuelle complète,
  **droit de rétractation 14 jours** en vente à distance aux consommateurs, **garantie légale de conformité
  (2 ans neuf / au minimum 1 an occasion)**, médiateur de la consommation, prix TTC (guide vendeur DGCCRF).
  Sur Vinted entre particuliers, ces obligations « pro » ne s'appliquent pas de la même façon — le CDC impose donc
  de **déclarer par article son régime** (personnel/pro) plutôt que de mélanger les chiffres.
- **Étiquetage textile** : composition en fibres **obligatoire** dans l'UE (règlement UE n° 1007/2011, dénominations
  officielles, % décroissants) — d'où le champ `composition` obligatoire d'EF-VIN-01 ; l'affichage d'une origine doit être exact.
- ⚠️ Ce CDC n'est pas un conseil juridique/fiscal : **faire valider votre régime (§20-Q2) par votre expert-comptable**.

## 12. Intégrations

| ID | Intégration | Mode v1 | Priorité |
|---|---|---|---|
| INT-01 | E-mail (envoi devis/factures, codes de connexion, relances) | SMTP du serveur interne | M |
| INT-02 | Plateforme agréée facturation électronique | À choisir avant J6 ; réception dès v1 | M (réception) / S (émission) |
| INT-03 | Excel / CSV (import clients, articles, ventes Vinted ; exports partout) | Fichiers locaux | M |
| INT-04 | Stripe (liens de paiement sur factures, suivi) | API, jetons en coffre | S |
| INT-05 | Banque (relevés CSV/OFX pour rapprochement) | Import fichier | S |
| INT-06 | Signature électronique (devis) | Prestataire externe | C (v1.x) |
| INT-07 | SMS / notifications (relances) | Prestataire externe | C (v1.x) |

## 13. Migration / reprise depuis OpenConcerto & Dolibarr
Inventaire à J0 (ce qui existe réellement : clients, factures ouvertes, articles) → exports CSV des deux outils →
scripts d'import **rejouables** avec rapport d'écarts (doublons clients, trous de numérotation signalés, jamais corrigés en silence) →
bascule : les anciennes factures restent en lecture seule importées ; la nouvelle numérotation démarre proprement.
*Règle : aucune donnée historique n'est écrasée ; tout import est journalisé (SEC-05).*

## 14. Déploiement, DevOps, exploitation
Serveur interne Linux : PostgreSQL + (plus tard) `erp-server` en Docker, CLI sur votre poste. Environnements **staging + production** (vos choix) ; CI : `cargo test` + lints à chaque commit, pas de merge si rouge. **Sauvegarde quotidienne** automatisée (dump chiffré, rétention 30 j + 1 copie hors du serveur), **test de restauration à chaque jalon**. Mises à jour : binaire versionné, migrations de base automatiques et réversibles, procédure de retour arrière documentée. Supervision minimale : `erp /admin/sante` (base, espace disque, dernière sauvegarde).

## 15. Stratégie de test & recette (votre validation à chaque jalon)
1. **Tests unitaires** : 100 % des règles de calcul (TVA, totaux, marge Vinted, numérotation) — `erp-core` uniquement.
2. **Tests d'intégration** : parcours complets sur base de test (devis→facture→paiement→écriture→FEC).
3. **Recette par jalon (UAT)** : vous exécutez un **scénario écrit** fourni avec chaque livrable (ex. « créer le client X,
   facturer 2 h de maintenance, marquer payé, vérifier l'écriture et le PDF ») + les chronométrages OBJ-1/OBJ-2.
4. **Recette conformité** avant go-live : checklist §11 (mentions facture, FEC généré et lisible, export RGPD, restauration de sauvegarde).
5. Critère de sortie : 0 anomalie bloquante, anomalies mineures listées et acceptées par écrit par vous.

## 16. Planning indicatif (délai visé : première version en quelques semaines) — **chaque jalon attend votre GO**
| Jalon | Contenu | Livrable à valider |
|---|---|---|
| **J0** — Cadrage | Validation du présent CDC, réponses au §20, inventaire migration | Ce document signé (GO/NO-GO) |
| **J1** — Socle CLI | Navigation par chemins, raccourcis, sociétés/devises, PostgreSQL, audit, sauvegarde | Démo : OBJ-2 et OBJ-3 mesurés |
| **J2** — CRM + Facturation | Devis/factures/avoirs/PDF/e-mail (module critique d'abord) | Scénario recette facturation complet |
| **J3** — Comptabilité | Écritures, TVA, FEC, tableau de bord | FEC de test validé (outil DGFiP) |
| **J4** — Vinted | Articles, ventes, marges, alertes DAC7, import CSV | Import de vos ventes réelles |
| **J5** — Durcissement | 2FA, rôles, recette conformité §11/§10, restauration testée, migration Dolibarr/OpenConcerto | Checklist conformité signée → **mise en production** |
| **J6** — Avant 09/2027 | Émission e-factures via plateforme agréée | Premières e-factures émises/reçues |

## 17. Budget & TCO (budget « minimal »)
Développement : vous + assistant IA (pas de licence). Logiciels : 100 % open source (Rust, PostgreSQL) = **0 €**.
Hébergement : serveur interne existant = **0 € récurrent**. Coûts externes possibles : plateforme agréée
facturation électronique (souvent quelques €/mois ou incluse chez l'expert-comptable), nom de domaine/SMTP si besoin,
prestataires signature/SMS seulement en v1.x. TCO 3 ans estimé : **quasi nul hors temps de développement** — conforme à votre pondération (coût noté 0/5 mais budget déclaré minimal : le CDC retient le moins cher *à maintenance égale*, cf. simplicité 5/5).

## 18. Risques principaux
| Risque | Impact | Parade dans ce CDC |
|---|---|---|
| Vous êtes seul (bus factor = 1) | Maintenance bloquée | Simplicité 5/5, doc en français, binaire unique, exports ouverts (ENF-PORT-01) |
| Rust : développement plus lent qu'un langage « rapide » | Délai « quelques semaines » tendu | Périmètre v1 strict (§4.2), modules Should/Could décalables sans toucher au Must |
| Régime Vinted mal identifié (perso vs pro) | Redressement fiscal | Régime par article (EF-VIN-02), validation expert-comptable (§20-Q2) |
| Échéance e-facture 2027 ratée | Factures non conformes | J6 dédié, plateforme choisie dès J2 |
| Perte du serveur interne | Perte de données | Sauvegarde quotidienne + copie externe + test de restauration à chaque jalon |
| Périmètre qui gonfle (projets, commandes…) | Jamais livré | Won't list §4.2 ; tout ajout = avenant écrit au CDC |

## 19. Critères d'acceptation du projet (le CDC est « rempli » si…)
✅ OBJ-1 à OBJ-6 mesurés et atteints en recette · ✅ 100 % des exigences Must livrées et testées ·
✅ checklist conformité §11 validée avec l'expert-comptable · ✅ restauration de sauvegarde démontrée ·
✅ migration des données ouvertes effectuée sans perte · ✅ documentation d'installation/exploitation en français livrée.

## 20. Points ouverts — **à trancher par vous avant/pendant J0** (je ne coderai pas par-dessus ces trous)
| # | Question | Pourquoi c'est bloquant | Options |
|---|---|---|---|
| # | Question | Statut |
|---|---|---|
| Q1 | Statut juridique & TVA | ✅ **Tranché : micro-entreprise** (§21.1) |
| Q2 | Vinted : personnel ou achat-revente | ✅ **Tranché : achat-revente** (§21.2) |
| Q3 | TVA encaissement/débits, clients pros/particuliers | ✅ **Tranché après recherche : franchise en base tant qu'éligible, sinon encaissements** (§21.3) |
| Q4 | Plateforme agréée e-factures (open source) | 🕓 **Reporté au jalon UI web**, shortlist open source (§21.4) |
| Q5 | Reprise des données | ✅ **Exhaustive visée, outil reporté hors v1** (§21.5) |
| Q6 | Multi-sociétés en v1 | ✅ **Oui** — 1 micro légale, 2 activités séparées (§21.6) |

---

## Annexe A — Glossaire (court)
**FEC** fichier des écritures comptables (format DGFiP) · **PDP/plateforme agréée** intermédiaire immatriculé pour les e-factures ·
**DAC7** directive UE obligeant les plateformes à signaler les vendeurs (30 ventes ou 2 000 €/an) · **BIC** bénéfices industriels et commerciaux ·
**MoSCoW** priorisation Must/Should/Could/Won't · **UAT/recette** validation par l'utilisateur final · **TCO** coût total sur la durée de vie.

## Annexe B — Sources (sûres, vérifiées le 30/10/2026)
- Micro-entreprise, activités mixtes et seuils (régime micro 203 100 € / 83 600 € ; franchise en base TVA 37 500 € / 85 000 € en 2026) : entreprendre.service-public.fr (F23267) et synthèses conformes au CGI art. 50-0 et 293 B.
- TVA des prestations de services (exigibilité à l'encaissement par défaut, option débits possible, biens à la livraison) : CGI art. 269 et guides fiscaux concordants.
- Écosystème facturation électronique open source (Odoo, Axelor parmi les plateformes agréées DGFiP) : comparateurs du registre DGFiP, octobre 2026 — à revalider au choix définitif.
- Facturation électronique, calendrier officiel (réception 01/09/2026 toutes entreprises ; émission TPE/PME 01/09/2027, plateforme agréée, nouvelles mentions) : economie.gouv.fr — « Tout savoir sur la facturation électronique pour les entreprises ».
- Mentions obligatoires des factures de prestation de service (CGI 242 nonies A, pénalités, indemnité 40 €) : guides conformes au CGI/Code de commerce (Fiducial ; L-Expert-Comptable).
- FEC et contrôles (format normé, art. A.47 AA-1 LPF) : service-public.fr (F34418).
- RGPD (durées par finalité, droits, réponse sous 1 mois) : cnil.fr.
- Vente de vêtements / Vinted (occasionnel non imposable, achat-revente = BIC, signalement DAC7 30 ventes/2 000 € = signalement ≠ imposition) : service-public.fr / DGFiP repris par la presse spécialisée ; frontière pro = achat en vue de revendre, activité habituelle.
- Vente à distance professionnelle (rétractation 14 j, garantie de conformité 2 ans neuf / ≥ 1 an occasion, médiation) : DGCCRF, « Guide du vendeur e-commerce » (economie.gouv.fr).
- Étiquetage textile (composition obligatoire, règlement UE n° 1007/2011) : synthèse conforme DGCCRF/UE.
- Méthode CDC : modèle d'exigences Volere ; ISO/IEC 25010:2023 (9 caractéristiques qualité) pour les exigences non fonctionnelles.

## Annexe C — Traçabilité (principe)
Chaque exigence `EF/ENF/SEC/RGPD/INT` de ce CDC sera reprise dans le dépôt avec : statut, jalon (§16), test associé, date de validation par vous. La matrice complète est générée au démarrage de J1.

## 21. Addendum du 30/10/2026 — vos réponses Q1–Q6 intégrées

### 21.1 Q1 — Micro-entreprise (statut tranché)
Une seule micro-entreprise (un seul SIREN/SIRET : on ne peut pas cumuler deux micro-entreprises), avec
**activité mixte** : ventes de marchandises (achat-revente vêtements) + prestations de services informatiques.
Conséquences pour l'ERP : (a) suivi du chiffre d'affaires **séparé par activité** avec les seuils 2026
(activité mixte : CA global ≤ 203 100 € dont ≤ 83 600 € de prestations de services — seuils du régime micro) ;
(b) mentions de facture du régime micro (§11.1) ; (c) tableau de bord « distance aux seuils » (nouvelle exigence
**EF-CPT-06 (S)** : `erp /compta/seuils` affiche CA ventes, CA services, % des seuils micro et franchise TVA).

### 21.2 Q2 — Vinted = achat-revente (mode professionnel dès la v1)
EF-VIN-02 s'applique en mode **pro BIC** : chaque article a un prix d'achat, la marge est calculée nette de frais,
les recettes alimentent le CA « ventes » de la micro-entreprise, alertes DAC7 (30 ventes / 2 000 €) activées par défaut.
Le mode « biens personnels » reste possible par article (utile si vous vendez un jour vos propres affaires), mais
**le défaut est PRO**. À valider avec l'expert-comptable : immatriculation et catégorie déclarative exactes.

### 21.3 Q3 — TVA (recherché pour vous, clients pros ET particuliers)
- **En micro-entreprise, tant que vous restez sous les seuils de franchise en base** (2026 : 37 500 € de CA services /
  85 000 € de CA ventes, seuils majorés 41 250 € / 93 500 €), vous ne facturez **pas de TVA** : mention obligatoire
  « TVA non applicable, art. 293 B du CGI ». Le débat encaissement/débits ne se pose donc pas aujourd'hui.
- **Si vous devenez redevable de la TVA** (dépassement ou option), la règle légale par défaut est :
  prestations de services = **TVA exigible à l'encaissement** (vous ne reversez que la TVA réellement encaissée —
  le plus favorable pour votre trésorerie, surtout avec des particuliers qui ne récupèrent pas la TVA) ;
  ventes de biens = exigible à la **livraison**. L'option « débits » (TVA dès la facture) n'a d'intérêt que si vos
  clients pros veulent déduire leur TVA avant de vous payer : **non retenue par défaut**.
- **Exigence ERP** : le moteur de TVA gère dès la v1 les deux mondes — mode *franchise* (0 %) et mode *TVA*
  (taux par ligne, exigibilité différenciée services/biens, bascule documentée si franchissement de seuil),
  et les deux clientèles : particulier (prix TTC affichés, facture sur demande/≥ 25 €) et pro (facture obligatoire,
  TVA intracom. et autoliquidation UE si applicable).

### 21.4 Q4 — Facturation électronique open source (reporté au jalon UI web, à votre demande)
Décision différée, mais l'architecture la rend indolore : l'ERP génère les formats structurés (**Factur-X / UBL / CII**)
et transmet via un **connecteur interchangeable** (`INT-02`) — le choix de la plateforme se fera quand vous ferez l'UI web,
sans toucher au module Facturation. Pistes open source relevées (registre DGFiP évolutif, **à revalider au moment du choix**) :
**Odoo** et **Axelor**, ERP open source figurant parmi les plateformes agréées, ou un connecteur API vers une plateforme
agréée distincte si vous préférez garder cet ERP-ci. Rappel des échéances : réception déjà obligatoire (depuis le
01/09/2026, donc à couvrir dès qu'un connecteur est branché), émission pour les micro/TPE au 01/09/2027.

### 21.5 Q5 — Migration : tout garder, mais plus tard
Objectif acté : reprise **exhaustive** (clients, historique, articles, pièces) « afin de ne rien perdre ».
L'**outil de migration sort du périmètre v1** (reporté) ; en contrepartie, la v1 garantit : modèle de données complet,
exports ouverts totaux (ENF-PORT-01) et identifiants d'origine conservables, pour que la migration future soit
un simple import sans perte. Le §13 s'appliquera le jour venu (imports rejouables, rapport d'écarts).

### 21.6 Q6 — Multi-sociétés : OUI, configuré juste juridiquement
La fonction multi-sociétés (EF-SOC-01) est **activée dès la v1**. Configuration initiale conforme à votre statut :
**1 société légale (votre micro-entreprise) avec 2 activités analytiques** — « Services informatiques » et
« Achat-revente vêtements » — chacune avec ses séries de factures, son CA et ses seuils (§21.1), devises gérées
(EF-SOC-02). Si vous créez un jour une vraie seconde société, le même mécanisme l'accueillera sans refonte.

*Fin du CDC v1.1 — prochaine et dernière action avant code : votre **GO explicite sur le J0** (ou vos corrections). Ensuite J1 (socle CLI), validé par vous avant J2.*
