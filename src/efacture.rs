//! Facturation électronique : export structuré UBL 2.1 (EF-FAC-07).
//!
//! La réforme impose la transmission via une **plateforme agréée** ; l'ERP ne
//! prétend pas l'être (CDC §4.2). Ce qu'il doit savoir faire dès la v1, c'est
//! produire un **format structuré normalisé** lisible par n'importe quelle
//! plateforme : c'est le rôle de ce module (profil EN 16931, type 380 facture,
//! 381 avoir, 384 devis ramené à une facture pro forma).
//!
//! Le connecteur de transmission reste interchangeable (`INT-02`) : brancher
//! une plateforme = envoyer ces octets, sans toucher au module Facturation.

use crate::domain::*;

fn xml_echapper(texte: &str) -> String {
    texte
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Montant au format UBL : deux décimales, point séparateur.
fn montant_ubl(cents: Cents) -> String {
    let signe = if cents < 0 { "-" } else { "" };
    format!(
        "{signe}{}.{:02}",
        cents.abs() / 100,
        cents.abs() % 100
    )
}

fn quantite_ubl(milli: i64) -> String {
    format!("{}.{:03}", milli / 1000, (milli % 1000).abs())
}

fn code_type(document: &Document) -> &'static str {
    match document.type_doc {
        TypeDocument::Avoir => "381",
        TypeDocument::Devis => "384",
        TypeDocument::Facture => "380",
    }
}

fn adresse_xml(client: &Client) -> String {
    format!(
        "      <cac:PostalAddress>\n\
         {}\
         {}\
         {}\
         <cac:Country><cbc:IdentificationCode>{}</cbc:IdentificationCode></cac:Country>\n\
         </cac:PostalAddress>\n",
        if client.adresse.is_empty() {
            String::new()
        } else {
            format!("        <cbc:StreetName>{}</cbc:StreetName>\n", xml_echapper(&client.adresse))
        },
        if client.ville.is_empty() {
            String::new()
        } else {
            format!("        <cbc:CityName>{}</cbc:CityName>\n", xml_echapper(&client.ville))
        },
        if client.code_postal.is_empty() {
            String::new()
        } else {
            format!("        <cbc:PostalZone>{}</cbc:PostalZone>\n", xml_echapper(&client.code_postal))
        },
        xml_echapper(if client.pays.is_empty() { "FR" } else { client.pays.as_str() })
    )
}

/// Facture (ou avoir) au format UBL 2.1, profil EN 16931.
pub fn ubl(societe: &Societe, client: &Client, document: &Document, regime: RegimeTva) -> String {
    let totaux = document.totaux(regime);
    let devise = if document.devise.is_empty() {
        DEVISE_REFERENCE
    } else {
        &document.devise
    };
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\" \
         xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\" \
         xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\">\n",
    );
    xml.push_str("  <cbc:UBLVersionID>2.1</cbc:UBLVersionID>\n");
    xml.push_str("  <cbc:CustomizationID>urn:cen.eu:en16931:2017</cbc:CustomizationID>\n");
    xml.push_str("  <cbc:ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</cbc:ProfileID>\n");
    xml.push_str(&format!("  <cbc:ID>{}</cbc:ID>\n", xml_echapper(&document.numero)));
    xml.push_str(&format!(
        "  <cbc:IssueDate>{}</cbc:IssueDate>\n",
        document.date_doc
    ));
    if date_valide(&document.echeance) {
        xml.push_str(&format!(
            "  <cbc:DueDate>{}</cbc:DueDate>\n",
            document.echeance
        ));
    }
    xml.push_str(&format!(
        "  <cbc:InvoiceTypeCode>{}</cbc:InvoiceTypeCode>\n",
        code_type(document)
    ));
    xml.push_str(&format!(
        "  <cbc:DocumentCurrencyCode>{}</cbc:DocumentCurrencyCode>\n",
        xml_echapper(devise)
    ));
    if !document.motif.is_empty() {
        xml.push_str(&format!(
            "  <cbc:Note>{}</cbc:Note>\n",
            xml_echapper(&document.motif)
        ));
    }

    // Émetteur
    xml.push_str("  <cac:AccountingSupplierParty>\n    <cac:Party>\n");
    xml.push_str(&format!(
        "      <cac:PartyName><cbc:Name>{}</cbc:Name></cac:PartyName>\n",
        xml_echapper(&societe.nom)
    ));
    xml.push_str(&format!(
        "      <cac:PostalAddress>\n        <cbc:StreetName>{}</cbc:StreetName>\n        \
         <cbc:CityName>{}</cbc:CityName>\n        <cbc:PostalZone>{}</cbc:PostalZone>\n        \
         <cac:Country><cbc:IdentificationCode>{}</cbc:IdentificationCode></cac:Country>\n      \
         </cac:PostalAddress>\n",
        xml_echapper(&societe.adresse),
        xml_echapper(&societe.ville),
        xml_echapper(&societe.code_postal),
        xml_echapper(if societe.pays.is_empty() { "FR" } else { societe.pays.as_str() })
    ));
    if !societe.siren.is_empty() {
        xml.push_str(&format!(
            "      <cac:PartyTaxScheme>\n        <cbc:CompanyID>{}</cbc:CompanyID>\n        \
             <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>\n      </cac:PartyTaxScheme>\n",
            xml_echapper(&societe.siren)
        ));
    }
    xml.push_str(&format!(
        "      <cac:PartyLegalEntity><cbc:RegistrationName>{}</cbc:RegistrationName>\
         <cbc:CompanyID>{}</cbc:CompanyID></cac:PartyLegalEntity>\n",
        xml_echapper(&societe.nom),
        xml_echapper(&societe.siren)
    ));
    xml.push_str("    </cac:Party>\n  </cac:AccountingSupplierParty>\n");

    // Client (SIREN client : nouvelle mention e-facture)
    xml.push_str("  <cac:AccountingCustomerParty>\n    <cac:Party>\n");
    xml.push_str(&format!(
        "      <cac:PartyName><cbc:Name>{}</cbc:Name></cac:PartyName>\n",
        xml_echapper(&client.nom)
    ));
    xml.push_str(&adresse_xml(client));
    if !client.siren.is_empty() {
        xml.push_str(&format!(
            "      <cac:PartyTaxScheme>\n        <cbc:CompanyID>{}</cbc:CompanyID>\n        \
             <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>\n      </cac:PartyTaxScheme>\n",
            xml_echapper(&client.siren)
        ));
    }
    xml.push_str("    </cac:Party>\n  </cac:AccountingCustomerParty>\n");

    // Totaux de TVA par taux
    xml.push_str("  <cac:TaxTotal>\n");
    xml.push_str(&format!(
        "    <cbc:TaxAmount currencyID=\"{}\">{}</cbc:TaxAmount>\n",
        xml_echapper(devise),
        montant_ubl(totaux.tva)
    ));
    let ventilation = tva_par_taux(&document.lignes, regime);
    if ventilation.is_empty() {
        xml.push_str(&format!(
            "    <cac:TaxSubtotal>\n      <cbc:TaxableAmount currencyID=\"{}\">{}</cbc:TaxableAmount>\n      \
             <cbc:TaxAmount currencyID=\"{}\">0.00</cbc:TaxAmount>\n      \
             <cac:TaxCategory><cbc:ID>E</cbc:ID><cbc:Percent>0</cbc:Percent>\
             <cbc:TaxExemptionReasonCode>VATEX-EU-132</cbc:TaxExemptionReasonCode>\
             <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory>\n    </cac:TaxSubtotal>\n",
            xml_echapper(devise),
            montant_ubl(totaux.ht),
            xml_echapper(devise)
        ));
    }
    for taux in &ventilation {
        xml.push_str(&format!(
            "    <cac:TaxSubtotal>\n      <cbc:TaxableAmount currencyID=\"{dev}\">{base}</cbc:TaxableAmount>\n      \
             <cbc:TaxAmount currencyID=\"{dev}\">{tva}</cbc:TaxAmount>\n      \
             <cac:TaxCategory><cbc:ID>{categorie}</cbc:ID><cbc:Percent>{pct}</cbc:Percent>\
             <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory>\n    </cac:TaxSubtotal>\n",
            dev = xml_echapper(devise),
            base = montant_ubl(taux.base_ht),
            tva = montant_ubl(taux.tva),
            categorie = if taux.taux_bp == 0 { "E" } else { "S" },
            pct = format!("{:.2}", f64::from(taux.taux_bp) / 100.0)
        ));
    }
    xml.push_str("  </cac:TaxTotal>\n");

    xml.push_str(&format!(
        "  <cac:LegalMonetaryTotal>\n    <cbc:LineExtensionAmount currencyID=\"{dev}\">{ht}</cbc:LineExtensionAmount>\n    \
         <cbc:TaxExclusiveAmount currencyID=\"{dev}\">{ht}</cbc:TaxExclusiveAmount>\n    \
         <cbc:TaxInclusiveAmount currencyID=\"{dev}\">{ttc}</cbc:TaxInclusiveAmount>\n    \
         <cbc:PrepaidAmount currencyID=\"{dev}\">{paye}</cbc:PrepaidAmount>\n    \
         <cbc:PayableAmount currencyID=\"{dev}\">{reste}</cbc:PayableAmount>\n  </cac:LegalMonetaryTotal>\n",
        dev = xml_echapper(devise),
        ht = montant_ubl(totaux.ht),
        ttc = montant_ubl(totaux.ttc),
        paye = montant_ubl(document.paye),
        reste = montant_ubl(document.reste(regime))
    ));

    for (index, ligne) in document.lignes.iter().enumerate() {
        xml.push_str("  <cac:InvoiceLine>\n");
        xml.push_str(&format!("    <cbc:ID>{}</cbc:ID>\n", index + 1));
        xml.push_str(&format!(
            "    <cbc:InvoicedQuantity unitCode=\"C62\">{}</cbc:InvoicedQuantity>\n",
            quantite_ubl(ligne.quantite_milli)
        ));
        xml.push_str(&format!(
            "    <cbc:LineExtensionAmount currencyID=\"{}\">{}</cbc:LineExtensionAmount>\n",
            xml_echapper(devise),
            montant_ubl(ligne.total_ht())
        ));
        xml.push_str(&format!(
            "    <cac:Item><cbc:Name>{}</cbc:Name></cac:Item>\n",
            xml_echapper(&ligne.designation)
        ));
        xml.push_str(&format!(
            "    <cac:Price><cbc:PriceAmount currencyID=\"{}\">{}</cbc:PriceAmount></cac:Price>\n",
            xml_echapper(devise),
            montant_ubl(ligne.prix_unitaire_ht)
        ));
        xml.push_str("  </cac:InvoiceLine>\n");
    }
    xml.push_str("</Invoice>\n");
    xml
}

/// Déclaration d'échange de biens / autoliquidation : indication de la nature
/// de l'opération, exigée par la réforme (mention « nature de l'opération »).
pub fn nature_operation(document: &Document) -> &'static str {
    match document.activite.as_str() {
        "ventes" => "livraison de biens",
        _ => "prestation de services",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn societe() -> Societe {
        Societe {
            nom: "Dupont Conseil & Cie".into(),
            siren: "123456789".into(),
            adresse: "1 rue de la Paix".into(),
            code_postal: "06000".into(),
            ville: "Nice".into(),
            ..Societe::default()
        }
    }

    fn client() -> Client {
        let mut c = Client::nouveau("Atelier <Dupont>", TypeClient::Professionnel);
        c.siren = "987654321".into();
        c.adresse = "5 avenue des Fleurs".into();
        c.ville = "Nice".into();
        c
    }

    fn facture() -> Document {
        Document {
            numero: "FA-2026-0001".into(),
            date_doc: "2026-10-30".into(),
            echeance: "2026-11-29".into(),
            lignes: vec![Ligne::new("Maintenance", 2.0, 50_000, 2000)],
            ..Document::default()
        }
    }

    #[test]
    fn ubl_contient_les_mentions_clefs() {
        let xml = ubl(&societe(), &client(), &facture(), RegimeTva::Reel);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<cbc:ID>FA-2026-0001</cbc:ID>"));
        assert!(xml.contains("<cbc:InvoiceTypeCode>380</cbc:InvoiceTypeCode>"));
        assert!(xml.contains("<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>"));
        assert!(xml.contains("<cbc:CompanyID>123456789</cbc:CompanyID>"));
        assert!(xml.contains("<cbc:CompanyID>987654321</cbc:CompanyID>"));
        assert!(xml.contains("<cbc:TaxAmount currencyID=\"EUR\">20.00</cbc:TaxAmount>"));
        assert!(xml.contains("<cbc:PayableAmount currencyID=\"EUR\">120.00</cbc:PayableAmount>"));
        assert!(xml.contains("<cbc:InvoicedQuantity unitCode=\"C62\">2.000</cbc:InvoicedQuantity>"));
    }

    #[test]
    fn ubl_echappe_le_xml() {
        let xml = ubl(&societe(), &client(), &facture(), RegimeTva::Reel);
        assert!(xml.contains("Atelier &lt;Dupont&gt;"));
        assert!(xml.contains("Dupont Conseil &amp; Cie"));
        assert!(!xml.contains("Atelier <Dupont>"));
    }

    #[test]
    fn ubl_franchise_exonere() {
        let xml = ubl(&societe(), &client(), &facture(), RegimeTva::Franchise);
        assert!(xml.contains("VATEX-EU-132"));
        assert!(xml.contains("<cbc:TaxAmount currencyID=\"EUR\">0.00</cbc:TaxAmount>"));
    }

    #[test]
    fn avoir_et_nature() {
        let mut doc = facture();
        doc.type_doc = TypeDocument::Avoir;
        let xml = ubl(&societe(), &client(), &doc, RegimeTva::Reel);
        assert!(xml.contains("<cbc:InvoiceTypeCode>381</cbc:InvoiceTypeCode>"));
        assert!(xml.contains("<cac:TaxSubtotal>"));
        assert_eq!(nature_operation(&doc), "prestation de services");
        doc.activite = "ventes".into();
        assert_eq!(nature_operation(&doc), "livraison de biens");
    }

    #[test]
    fn montants_ubl() {
        assert_eq!(montant_ubl(120_000), "1200.00");
        assert_eq!(montant_ubl(-5_050), "-50.50");
        assert_eq!(quantite_ubl(2_500), "2.500");
    }
}
