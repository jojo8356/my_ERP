# Décisions — Stack de l'ERP

- **`mes-choix.json`** : vos réponses au questionnaire (`../index.html`), source de vérité pour tous les développements.
  Statut actuel : ✅ réponses reçues le 30/10/2026 + réponses Q1–Q6 intégrées au CDC v1.1 (§21). En attente du **GO J0** de l'utilisateur avant tout code.
- **`../CHOIX-STACK-ERP.md`** : la méthode (comment les grandes entreprises choisissent) + le questionnaire en version texte.

## Règle de travail

1. Avant de coder un module, relire `mes-choix.json`.
2. Un choix explicite de l'utilisateur prime toujours sur une recommandation automatique.
3. Tout nouveau choix structurant = mise à jour de ce dossier (date + justification), commit Git.
