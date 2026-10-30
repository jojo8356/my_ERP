//! Thème visuel unique de l'ERP, entièrement construit avec Chromat
//! (https://github.com/jojo8356/chromat).
//!
//! Aucun code couleur ANSI ne doit être écrit ailleurs dans le projet :
//! les modules demandent un style sémantique à ce fichier, et Chromat se
//! charge de la détection du terminal (`NO_COLOR`, TTY, 16/256/truecolor)
//! ainsi que de la conversion de la couleur demandée.
//!
//! Les séquences de contrôle du terminal (effacer l'écran, écran alternatif,
//! afficher/masquer le curseur) ne sont pas des couleurs : elles restent dans
//! `tui.rs`, qui est le seul module autorisé à manipuler le terminal.

use chromat::{Colorize, NamedColors, StyledString};

/// Palette partagée avec Ratatui (ADR-005) : la TUI ne consomme pas les
/// chaînes ANSI de Chromat, elle applique ces mêmes valeurs RGB à ses
/// styles. Le CLI, lui, passe directement par les fonctions Chromat
/// ci-dessous. Seul fond autorisé : le surlignage blanc à 20 % de la
/// sélection, demandé explicitement par l'utilisateur (voir ci-dessous).
pub const BLANC_RGB: (u8, u8, u8) = (255, 255, 255);
/// Équivalent terminal d'un blanc à 20 % d'opacité sur fond noir :
/// Ratatui/ANSI ne gèrent pas la transparence, donc 20 % de blanc
/// (255 × 0,20 ≈ 51) sur les trois canaux.
pub const SURLIGNAGE_BLANC_20_RGB: (u8, u8, u8) = (51, 51, 51);
pub const VIOLET_RGB: (u8, u8, u8) = (238, 130, 238);
pub const VERT_RGB: (u8, u8, u8) = (0, 128, 0);
pub const ORANGE_RGB: (u8, u8, u8) = (255, 165, 0);
pub const ROUGE_RGB: (u8, u8, u8) = (255, 0, 0);
pub const BLEU_RGB: (u8, u8, u8) = (30, 144, 255);

/// Texte blanc (cadres, séparateurs, libellés neutres).
pub fn blanc(texte: impl Into<String>) -> StyledString {
    texte.into().white()
}

/// Titre : blanc en gras.
pub fn titre(texte: impl Into<String>) -> StyledString {
    texte.into().white().bold()
}

/// Écriture violette du menu (onglets inactifs).
pub fn violet(texte: impl Into<String>) -> StyledString {
    texte.into().violet()
}

/// Sélection (onglet actif, ligne choisie) : violet en gras.
pub fn selection(texte: impl Into<String>) -> StyledString {
    texte.into().violet().bold()
}

/// Succès (créations, validations, paiements, contrôles réussis).
pub fn succes(texte: impl Into<String>) -> StyledString {
    texte.into().green()
}

/// Point d'attention (alertes de seuils, impayés, DAC7).
pub fn alerte(texte: impl Into<String>) -> StyledString {
    texte.into().orange()
}

/// Erreur affichée à l'utilisateur.
pub fn erreur(texte: impl Into<String>) -> StyledString {
    texte.into().red().bold()
}

/// Information secondaire ou aide contextuelle.
pub fn info(texte: impl Into<String>) -> StyledString {
    texte.into().dodgerblue()
}

/// Chromat conserve le niveau de couleur dans un état global au processus.
/// Les tests qui le forcent doivent donc se sérialiser avec ce verrou.
#[cfg(test)]
pub(crate) static VERROU_COULEUR: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use chromat::{reset_color_level, set_color_level, ColorLevel};

    #[test]
    fn chromat_produit_les_couleurs_du_theme() {
        let _garde = VERROU_COULEUR.lock().unwrap();
        // Chromat détecte automatiquement le niveau du terminal ; on force
        // le truecolor ici pour vérifier déterministiquement les couleurs.
        set_color_level(ColorLevel::TrueColor);
        let rendu_violet = violet("menu").to_string();
        let rendu_selection = selection("actif").to_string();
        let rendu_titre = titre("titre").to_string();
        reset_color_level();

        // CSS `violet` = RGB(238, 130, 238), émis en truecolor par Chromat.
        assert!(rendu_violet.contains("\u{1b}[38;2;238;130;238m"), "{rendu_violet}");
        assert!(rendu_selection.contains("\u{1b}[38;2;238;130;238m"), "{rendu_selection}");
        assert!(rendu_selection.contains("\u{1b}[1m"), "gras manquant : {rendu_selection}");
        assert!(rendu_titre.contains("\u{1b}[38;2;255;255;255m"), "{rendu_titre}");
        // Aucun style du thème ne doit produire de fond coloré.
        assert!(!rendu_violet.contains("\u{1b}[48;"));
        assert!(!rendu_titre.contains("\u{1b}[48;"));
    }

    #[test]
    fn chromat_respecte_no_color_via_son_niveau() {
        let _garde = VERROU_COULEUR.lock().unwrap();
        set_color_level(ColorLevel::NoColor);
        let rendu = violet("menu").to_string();
        reset_color_level();
        assert_eq!(rendu, "menu");
    }
}
