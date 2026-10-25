//! The Home view: two path fields and Compare (UI-001).

use dioxus::prelude::*;

use crate::app::ViewModel;
use crate::commands::CommandId;
use crate::home::HomeModel;
use crate::ui::Ui;

/// Sets a field of the active Home view.
fn set_field(ui: Ui, right: bool, value: String) {
    ui.act(|app| {
        if let Some(tab) = app.active_tab_mut()
            && let ViewModel::Home(home) = &mut tab.model
        {
            if right {
                home.right = value;
            } else {
                home.left = value;
            }
            home.error = None;
        }
        Vec::new()
    });
}

/// The Home pane.
#[component]
pub fn HomePane(home: HomeModel) -> Element {
    let ui = use_context::<Ui>();
    let ready = home.ready() && !home.resolving();
    let on_key = move |evt: KeyboardEvent| {
        if evt.key() == Key::Enter {
            evt.prevent_default();
            ui.command(CommandId::HomeCompare);
        }
    };
    rsx! {
        div { class: "hk-home",
            div { class: "hk-home-card",
                h1 { class: "hk-home-title", "Hikasa" }
                p { class: "hk-home-hint",
                    "Compare two folders or two files. Paths may be relative to the folder Hikasa was started in."
                }
                label { class: "hk-field",
                    span { "Left" }
                    input {
                        r#type: "text",
                        value: "{home.left}",
                        placeholder: "/path/to/left",
                        spellcheck: "false",
                        oninput: move |evt: FormEvent| set_field(ui, false, evt.value()),
                        onkeydown: on_key,
                    }
                }
                label { class: "hk-field",
                    span { "Right" }
                    input {
                        r#type: "text",
                        value: "{home.right}",
                        placeholder: "/path/to/right",
                        spellcheck: "false",
                        oninput: move |evt: FormEvent| set_field(ui, true, evt.value()),
                        onkeydown: on_key,
                    }
                }
                div { class: "hk-home-actions",
                    button {
                        class: "hk-primary",
                        disabled: !ready,
                        onclick: move |_| ui.command(CommandId::HomeCompare),
                        "Compare"
                    }
                }
                if home.resolving() {
                    div { class: "hk-home-hint", "Opening…" }
                }
                if let Some(error) = &home.error {
                    div { class: "hk-error", "{error}" }
                }
            }
        }
    }
}
