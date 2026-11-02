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
/// Violet atténué des onglets inactifs : 70 % du violet actif, pour une
/// hiérarchie lisible sans écraser l'onglet sélectionné.
pub const VIOLET_INACTIF_RGB: (u8, u8, u8) = (167, 91, 167);
/// Couleurs sémantiques recalées pour fond sombre : les verts/oranges/
/// rouges/bleus CSS de base sont trop sombres ou trop saturés sur noir
/// et vibrent contre le violet. On utilise une même échelle tonale
/// claire et désaturée (niveau « 400 » des design systems modernes) :
/// luminosité homogène, contraste lisible, aucune couleur primaire pure.
pub const VERT_RGB: (u8, u8, u8) = (74, 222, 128); // succès — #4ADE80
pub const ORANGE_RGB: (u8, u8, u8) = (251, 191, 36); // alerte — #FBBF24
pub const ROUGE_RGB: (u8, u8, u8) = (248, 113, 113); // erreur — #F87171
pub const BLEU_RGB: (u8, u8, u8) = (96, 165, 250); // info — #60A5FA

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
    texte.into().fg(VERT_RGB)
}

/// Point d'attention (alertes de seuils, impayés, DAC7).
pub fn alerte(texte: impl Into<String>) -> StyledString {
    texte.into().fg(ORANGE_RGB)
}

/// Erreur affichée à l'utilisateur.
pub fn erreur(texte: impl Into<String>) -> StyledString {
    texte.into().fg(ROUGE_RGB).bold()
}

/// Information secondaire ou aide contextuelle.
pub fn info(texte: impl Into<String>) -> StyledString {
    texte.into().fg(BLEU_RGB)
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
        let rendu_succes = succes("ok").to_string();
        let rendu_alerte = alerte("attention").to_string();
        let rendu_erreur = erreur("erreur").to_string();
        let rendu_info = info("info").to_string();
        reset_color_level();

        // CSS `violet` = RGB(238, 130, 238), émis en truecolor par Chromat.
        assert!(rendu_violet.contains("\u{1b}[38;2;238;130;238m"), "{rendu_violet}");
        assert!(rendu_selection.contains("\u{1b}[38;2;238;130;238m"), "{rendu_selection}");
        assert!(rendu_selection.contains("\u{1b}[1m"), "gras manquant : {rendu_selection}");
        assert!(rendu_titre.contains("\u{1b}[38;2;255;255;255m"), "{rendu_titre}");
        // Aucun style du thème ne doit produire de fond coloré.
        // Sémantiques fond sombre : échelle tonale claire homogène.
        assert!(rendu_succes.contains("\u{1b}[38;2;74;222;128m"), "{rendu_succes}");
        assert!(rendu_alerte.contains("\u{1b}[38;2;251;191;36m"), "{rendu_alerte}");
        assert!(rendu_erreur.contains("\u{1b}[38;2;248;113;113m"), "{rendu_erreur}");
        assert!(rendu_info.contains("\u{1b}[38;2;96;165;250m"), "{rendu_info}");
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
