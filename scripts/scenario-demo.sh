#!/usr/bin/env bash
# Scénario de démonstration / recette de la V1 (utilisé par la CI et en local).
# Usage : ERP_DONNEES_DIR=/tmp/erp-demo bash scripts/scenario-demo.sh
# Requiert le binaire déjà compilé (cargo build) ou utilise `cargo run`.
set -euo pipefail

ERP="cargo run --quiet --"

echo "── 1. Création d'un client"
$ERP /clients nouveau --nom "Atelier Dupont" --type pro --email contact@example.fr

echo "── 2. Facture de maintenance (2 × 500,00 €)"
NUM=$($ERP /factures nouvelle --client dupont --activite services --ligne "Maintenance informatique;2;500,00;2000" | grep -oE "FA-[0-9]{4}-[0-9]{4}" | head -1)
echo "    facture créée : $NUM"

echo "── 3. Validation (la facture devient immuable)"
$ERP /factures "$NUM" valider

echo "── 4. Paiement intégral"
$ERP /factures "$NUM" payer
$ERP /factures "$NUM" | grep "payée"

echo "── 5. Achat-revente Vinted"
$ERP /vinted nouveau --marque Zara --description "Veste" --taille M --composition "100 % coton" --prix-achat 8,00
ID_ARTICLE=$($ERP /vinted | awk 'NR==1 {print $1}')
$ERP /vinted vendre "$ID_ARTICLE" --prix 25,00 --frais 2,00
$ERP /vinted stats | grep "marge nette"

echo "── 6. Comptabilité : seuils micro et FEC"
$ERP /compta seuils | grep "CA global"
$ERP /compta fec | head -3

echo "── 7. Persistance sur disque"
test -f "${ERP_DONNEES_DIR:-donnees}/etat.tsv" && echo "    etat.tsv présent : persistance OK"
echo "✅ Scénario terminé"
