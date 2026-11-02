#!/usr/bin/env bash
# Scénario de démonstration / recette de la V1 (utilisé par la CI et en local).
# Usage : ERP_DONNEES_DIR=/tmp/erp-demo bash scripts/scenario-demo.sh
# Requiert le binaire déjà compilé (cargo build) ou utilise `cargo run`.
set -euo pipefail

# En cas d'échec, la ligne fautive est écrite dans le journal (la CI la remonte
# en annotation : le journal complet n'est lisible que sur GitHub).
trap 'echo "✗ échec du scénario à la ligne ${LINENO} : ${BASH_COMMAND}" >&2' ERR

# `ERP_BIN` permet de rejouer la recette avec un binaire déjà compilé (mêmes
# options, notamment `--features postgres`) : sans lui, `cargo run` recompile et
# écraserait le binaire utilisé par l'appelant (cf. test-all.sh).
ERP="${ERP_BIN:-cargo run --quiet --}"
export ERP_DONNEES_DIR="${ERP_DONNEES_DIR:-donnees}"
DONNEES="$ERP_DONNEES_DIR"
mkdir -p "$DONNEES/exports"

echo "── 0. Identité de l'émetteur (mentions obligatoires, CDC §11.1)"
$ERP /societe modifier --nom "Atelier Dupont" --siren 123456789 --adresse "1 rue de la Paix" \
  --cp 06000 --ville Nice --email contact@example.fr --forme micro-entreprise >/dev/null
$ERP /admin mentions | sed -n '1,4p'

echo "── 1. Création d'un client"
$ERP /clients nouveau --nom "Atelier Dupont" --type pro --email contact@example.fr --ville Nice

echo "── 2. Facture de maintenance (2 × 500,00 € à 20 %)"
NUM=$($ERP /factures nouvelle --client dupont --activite services --ligne "Maintenance informatique;2;500,00;2000" | grep -oE "FA-[0-9]{4}-[0-9]{4}" | sed -n '1,1p')
echo "    facture créée : $NUM"

echo "── 3. Pièces : PDF conforme + e-facture UBL (EN 16931)"
$ERP /factures "$NUM" pdf "$DONNEES/exports/$NUM.pdf"
$ERP /factures "$NUM" ubl "$DONNEES/exports/$NUM.xml"
test -s "$DONNEES/exports/$NUM.pdf"
test -s "$DONNEES/exports/$NUM.xml"

echo "── 4. Validation (contrôle des mentions puis pièce figée)"
$ERP /factures "$NUM" valider

echo "── 5. Envoi au client (message RFC 5322 + PDF joint, boîte locale)"
$ERP /factures "$NUM" envoyer | grep "courriels"

echo "── 6. Paiement intégral"
$ERP /factures "$NUM" payer
$ERP /factures "$NUM" | grep "payée"

echo "── 7. Achat-revente Vinted (marge nette = prix - frais - achat)"
$ERP /vinted nouveau --marque Zara --description "Veste" --taille M --composition "100 % coton" --prix-achat 8,00
ID_ARTICLE=$($ERP /vinted | awk 'NR==1 {print $1}')
$ERP /vinted vendre "$ID_ARTICLE" --prix 25,00 --frais 2,00
$ERP /vinted stats | grep "marge nette"

echo "── 8. Comptabilité : seuils micro, FEC, balance, TVA, tableau de bord"
$ERP /compta seuils | grep "CA global"
$ERP /compta fec | sed -n '1,3p'
$ERP /compta fec "$DONNEES/exports/fec.csv" >/dev/null
$ERP /compta journal | sed -n '1,4p'
$ERP /compta balance | sed -n '1,5p'
$ERP /compta tva | sed -n '1,5p'
$ERP /compta tableau | sed -n '1,5p'

echo "── 9. Rapprochement bancaire (relevé CSV puis lettrage automatique)"
printf 'date;libelle;montant\n%s;VIREMENT ATELIER DUPONT;1000,00\n' "$(date +%Y-%m-%d)" > "$DONNEES/exports/releve.csv"
$ERP /compta rapprocher "$DONNEES/exports/releve.csv" | grep "importée"
$ERP /compta lettrer | grep "rapproché"

echo "── 10. Devis accepté puis transformé en facture"
DEVIS=$($ERP /devis nouveau --client dupont --activite services --ligne "Audit;1;300,00;2000" | grep -oE "DEV-[0-9]{4}-[0-9]{4}" | sed -n '1,1p')
$ERP /devis "$DEVIS" envoyer >/dev/null
$ERP /devis "$DEVIS" accepter
$ERP /devis "$DEVIS" transformer | grep "transformé"

echo "── 11. Recherche globale et aide contextuelle"
$ERP /recherche dupont | sed -n '1,5p'
$ERP /aide factures | sed -n '1,3p'

echo "── 12. Sécurité : admin + 2FA obligatoire, session, journal d'audit chaîné"
$ERP /utilisateurs nouveau admin --role admin --mot-de-passe "MotDePasse-Local-2026" | grep "Compte"
# SEC-02 : la 2FA est obligatoire pour un administrateur. On active le TOTP et
# on récupère un code de secours à usage unique (affiché une seule fois).
SORTIE_2FA=$($ERP /utilisateurs 2fa admin)
echo "$SORTIE_2FA" | grep "secret"
CODE=$(echo "$SORTIE_2FA" | grep -Eo '^[[:space:]]+[0-9a-f]{4}-[0-9a-f]{4}' | sed -n '1,1p' | tr -d ' ')
test -n "$CODE"
$ERP /auth connexion admin --mot-de-passe "MotDePasse-Local-2026" --totp "$CODE" | grep "Connecté"
$ERP /auth statut | grep "admin"
$ERP /admin audit --verifier | grep "aucune altération"
$ERP /auth deconnexion >/dev/null
# SEC-03 : second facteur envoyé par e-mail (empreinte seule, usage unique).
$ERP /utilisateurs email admin --email admin@exemple.fr | grep "Adresse e-mail"
$ERP /utilisateurs code admin | grep "envoyé à"
CODE_MAIL=$(grep -h "Voici votre code de connexion" "$DONNEES"/courriels/*.eml |
            tail -1 | sed 's/.* : //' | tr -d '\r ')
test "${#CODE_MAIL}" -eq 6
$ERP /auth connexion admin --mot-de-passe "MotDePasse-Local-2026" --code "$CODE_MAIL" |
  grep "code reçu par e-mail"
$ERP /auth deconnexion >/dev/null
# Rejouer le même code doit être refusé (usage unique).
if $ERP /auth connexion admin --mot-de-passe "MotDePasse-Local-2026" --code "$CODE_MAIL" \
     2>&1 | grep -q "Connecté"; then
  echo "✗ le code de connexion a été rejoué" >&2; exit 1
fi

echo "── 13. Devises : taux daté et conversion"
$ERP /devises taux USD 2026-01-15 0,92 | grep "Taux enregistré"
$ERP /devises convertir 100,00 USD 2026-01-15

echo "── 14. Multi-société : création et bascule"
$ERP /societes nouvelle --nom "Atelier Dupont SARL" --regime reel --ville Nice | grep "créée"
$ERP /societes | sed -n '1,3p'
$ERP /societes basculer "Atelier Dupont" | grep "courante"

echo "── 15. Sauvegarde et persistance sur disque"
$ERP /admin sauvegarde | grep -i "sauvegarde"
test -f "$DONNEES/meta.tsv" && echo "    meta.tsv présent (société courante, session, séquence)"
test -n "$(ls "$DONNEES"/tables/*.tsv 2>/dev/null | sed -n '1,1p')" && echo "    tables/*.tsv présentes : persistance OK"
$ERP /admin sante | sed -n '1,5p'

echo "── 16. Facture obligatoire ou non (CDC §11.2)"
$ERP /clients nouveau --nom "Camille Particulier" --type particulier >/dev/null
$ERP /factures nouvelle --client particulier --ligne "Dépannage;1;10,00" | grep "facultative"
$ERP /factures nouvelle --client particulier --ligne "Dépannage;1;40,00" | grep "facture obligatoire"
$ERP /factures nouvelle --client particulier --ligne "Dépannage;1;5,00" --sur-demande |
  grep "demandée"
echo "✅ Scénario terminé"
