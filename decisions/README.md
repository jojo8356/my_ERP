# Décisions — Stack de l'ERP

- **`mes-choix.json`** : vos réponses au questionnaire (`../index.html`), source de vérité pour tous les développements.
  Statut actuel : ✅ réponses reçues le 30/10/2026 + réponses Q1–Q6 intégrées au CDC v1.1 (§21). En attente du **GO J0** de l'utilisateur avant tout code.
- **`../CHOIX-STACK-ERP.md`** : la méthode (comment les grandes entreprises choisissent) + le questionnaire en version texte.

## Règle de travail

1. Avant de coder un module, relire `mes-choix.json`.
2. Un choix explicite de l'utilisateur prime toujours sur une recommandation automatique.
3. Tout nouveau choix structurant = mise à jour de ce dossier (date + justification), commit Git.

## Journal des ADR

| ADR | Sujet | Statut |
|---|---|---|
| [ADR-002](ADR-002-persistance-v1.md) | Persistance locale de la V1 (PostgreSQL différé) | acté, amendé par 006 |
| [ADR-003](ADR-003-tui.md) | TUI plein écran (Ratatui + Crossterm) | acté, complété par 005 |
| [ADR-004](ADR-004-chromat.md) | Toutes les couleurs via Chromat | acté |
| [ADR-005](ADR-005-ratatui-formulaires.md) | Formulaires interactifs et palette partagée | acté |
| [ADR-006](ADR-006-rustorm-sans-base.md) | Rustorm comme modèle unique, **sans base de données** | acté |
| [ADR-007](ADR-007-securite-locale.md) | Sécurité locale : rôles, PBKDF2, TOTP, audit chaîné | acté |
| [ADR-008](ADR-008-documents-pdf-email-efacture.md) | PDF, e-mail, CSV et e-facture écrits dans le dépôt | acté |

Traçabilité exigence par exigence du CDC v1.1 : [`../CONFORMITE-CDC.md`](../CONFORMITE-CDC.md).
