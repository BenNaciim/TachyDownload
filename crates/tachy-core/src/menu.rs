//! The window's menu bar as plain data (`menu.js`). Everything an entry does is an [`Action`]; the shell turns this
//! description into native menus and runs the action when an entry is chosen, so the layout can be checked without a
//! window.

pub const LIMITS: [(u64, &str); 6] = [
    (0, "Aucune limite"),
    (256, "256 Ko/s"),
    (512, "512 Ko/s"),
    (1024, "1 Mo/s"),
    (2048, "2 Mo/s"),
    (5120, "5 Mo/s"),
];

pub const AFTER: [(&str, &str); 4] = [
    ("none", "Ne rien faire"),
    ("quit", "Quitter l’application"),
    ("sleep", "Mettre l’ordinateur en veille"),
    ("shutdown", "Éteindre l’ordinateur"),
];

/// What an entry does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Shows the window and tells the page (`dm:menu`): `add`, `settings`, `search`, `theme` + argument.
    Ui(&'static str, Option<&'static str>),
    OpenFolder,
    Quit,
    PauseAll,
    ResumeAll,
    ClearFinished,
    SetLimit(u64),
    SetAfterAll(&'static str),
    CheckUpdate,
    CopyDiagnostic,
    OpenProject,
    About,
}

/// Built-in behaviours of the web view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    ZoomIn,
    ZoomOut,
    ResetZoom,
    ToggleFullscreen,
    ToggleDevTools,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Action {
        label: &'static str,
        accelerator: Option<&'static str>,
        action: Action,
        /// `Some` for a radio entry: whether it is the choice in force.
        checked: Option<bool>,
    },
    Role {
        label: &'static str,
        role: Role,
    },
    Separator,
    Submenu {
        label: &'static str,
        items: Vec<Item>,
    },
}

/// A top-level menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub label: &'static str,
    pub items: Vec<Item>,
}

fn act(label: &'static str, accelerator: Option<&'static str>, action: Action) -> Item {
    Item::Action {
        label,
        accelerator,
        action,
        checked: None,
    }
}

fn radio(label: &'static str, action: Action, checked: bool) -> Item {
    Item::Action {
        label,
        accelerator: None,
        action,
        checked: Some(checked),
    }
}

pub fn menu_template(speed_limit_kbps: u64, after_all: &str, dev_tools: bool) -> Vec<Menu> {
    let mut view = vec![
        act(
            "Rechercher",
            Some("CmdOrCtrl+F"),
            Action::Ui("search", None),
        ),
        Item::Separator,
        Item::Submenu {
            label: "Thème",
            items: vec![
                act("Automatique", None, Action::Ui("theme", Some("auto"))),
                act("Clair", None, Action::Ui("theme", Some("light"))),
                act("Sombre", None, Action::Ui("theme", Some("dark"))),
            ],
        },
        Item::Separator,
        Item::Role {
            label: "Zoom avant",
            role: Role::ZoomIn,
        },
        Item::Role {
            label: "Zoom arrière",
            role: Role::ZoomOut,
        },
        Item::Role {
            label: "Taille normale",
            role: Role::ResetZoom,
        },
        Item::Separator,
        Item::Role {
            label: "Plein écran",
            role: Role::ToggleFullscreen,
        },
    ];
    if dev_tools {
        view.push(Item::Separator);
        view.push(Item::Role {
            label: "Outils de développement",
            role: Role::ToggleDevTools,
        });
    }
    let mut limits: Vec<Item> = LIMITS
        .iter()
        .map(|(kbps, label)| radio(label, Action::SetLimit(*kbps), speed_limit_kbps == *kbps))
        .collect();
    limits.push(Item::Separator);
    limits.push(act("Autre valeur…", None, Action::Ui("settings", None)));
    vec![
        Menu {
            label: "Fichier",
            items: vec![
                act(
                    "Ajouter un lien…",
                    Some("CmdOrCtrl+N"),
                    Action::Ui("add", None),
                ),
                act(
                    "Ouvrir le dossier de téléchargement",
                    Some("CmdOrCtrl+Shift+O"),
                    Action::OpenFolder,
                ),
                Item::Separator,
                act(
                    "Paramètres",
                    Some("CmdOrCtrl+,"),
                    Action::Ui("settings", None),
                ),
                Item::Separator,
                act("Quitter", Some("CmdOrCtrl+Q"), Action::Quit),
            ],
        },
        Menu {
            label: "Téléchargements",
            items: vec![
                act(
                    "Tout mettre en pause",
                    Some("CmdOrCtrl+Shift+P"),
                    Action::PauseAll,
                ),
                act(
                    "Tout reprendre",
                    Some("CmdOrCtrl+Shift+R"),
                    Action::ResumeAll,
                ),
                act(
                    "Retirer les téléchargements terminés de la liste",
                    None,
                    Action::ClearFinished,
                ),
                Item::Separator,
                Item::Submenu {
                    label: "Limite de vitesse",
                    items: limits,
                },
                Item::Submenu {
                    label: "Quand tout est terminé",
                    items: AFTER
                        .iter()
                        .map(|(value, label)| {
                            radio(label, Action::SetAfterAll(value), after_all == *value)
                        })
                        .collect(),
                },
            ],
        },
        Menu {
            label: "Affichage",
            items: view,
        },
        Menu {
            label: "Aide",
            items: vec![
                act("Rechercher des mises à jour…", None, Action::CheckUpdate),
                act("Copier le diagnostic yt-dlp", None, Action::CopyDiagnostic),
                Item::Separator,
                act("Page du projet", None, Action::OpenProject),
                act("À propos de Tachy Download", None, Action::About),
            ],
        },
    ]
}

/// Every entry of a menu, submenus included, in order.
pub fn flatten(items: &[Item]) -> Vec<&Item> {
    let mut out = vec![];
    for i in items {
        out.push(i);
        if let Item::Submenu { items, .. } = i {
            out.extend(flatten(items));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(t: &[Menu]) -> Vec<&Item> {
        t.iter().flat_map(|m| flatten(&m.items)).collect()
    }

    fn by_label<'a>(items: &[&'a Item], wanted: &str) -> &'a Item {
        items
            .iter()
            .copied()
            .find(|i| match i {
                Item::Action { label, .. } | Item::Role { label, .. } => *label == wanted,
                _ => false,
            })
            .unwrap_or_else(|| panic!("no entry {wanted}"))
    }

    fn action_of(item: &Item) -> Action {
        match item {
            Item::Action { action, .. } => action.clone(),
            other => panic!("{other:?} is not an action"),
        }
    }

    #[test]
    fn the_menu_bar_offers_the_main_actions_and_each_entry_calls_the_right_thing() {
        let t = menu_template(1024, "quit", false);
        assert_eq!(
            t.iter().map(|m| m.label).collect::<Vec<_>>(),
            ["Fichier", "Téléchargements", "Affichage", "Aide"]
        );
        let items = all(&t);
        let chosen = |l: &str| action_of(by_label(&items, l));
        assert_eq!(chosen("Ajouter un lien…"), Action::Ui("add", None));
        assert_eq!(chosen("Paramètres"), Action::Ui("settings", None));
        assert_eq!(chosen("Rechercher"), Action::Ui("search", None));
        assert_eq!(chosen("Sombre"), Action::Ui("theme", Some("dark")));
        assert_eq!(chosen("Tout mettre en pause"), Action::PauseAll);
        assert_eq!(chosen("Tout reprendre"), Action::ResumeAll);
        assert_eq!(
            chosen("Retirer les téléchargements terminés de la liste"),
            Action::ClearFinished
        );
        assert_eq!(chosen("512 Ko/s"), Action::SetLimit(512));
        assert_eq!(
            chosen("Éteindre l’ordinateur"),
            Action::SetAfterAll("shutdown")
        );

        let checked = |l: &str| match by_label(&items, l) {
            Item::Action { checked, .. } => *checked,
            _ => None,
        };
        // the choice in force is ticked
        assert_eq!(checked("1 Mo/s"), Some(true));
        assert_eq!(checked("512 Ko/s"), Some(false));
        assert_eq!(checked("Quitter l’application"), Some(true));
        assert_eq!(checked("Ne rien faire"), Some(false));

        // developer tools only when asked for
        let has_dev = |t: &[Menu]| {
            all(t).iter().any(|i| {
                matches!(
                    i,
                    Item::Role {
                        role: Role::ToggleDevTools,
                        ..
                    }
                )
            })
        };
        assert!(!has_dev(&t));
        assert!(has_dev(&menu_template(0, "none", true)));
    }

    #[test]
    fn every_shortcut_in_the_menu_is_used_only_once() {
        let t = menu_template(0, "none", false);
        let keys: Vec<&str> = all(&t)
            .iter()
            .filter_map(|i| match i {
                Item::Action { accelerator, .. } => *accelerator,
                _ => None,
            })
            .collect();
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len());
    }
}
