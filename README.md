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

**[`CDC-ERP-CLI.md`](CDC-ERP-CLI.md)** — CDC complet v1.0 (ERP CLI Rust + PostgreSQL, conformité services informatiques & Vinted), en attente de validation (réponses au §20 + GO J0).

## Servir l'artefact en local

```bash
python3 -m http.server 8000
# puis http://localhost:8000
```
