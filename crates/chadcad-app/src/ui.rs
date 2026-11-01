//! The egui interface around the viewport.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;
use bevy_egui::egui::{self, Color32, RichText, TextEdit, Ui};
use chadcad_i18n::{I18n, LANGUAGES};
use chadcad_kernel::Format;
use chadcad_model::Expr;

use crate::editor;
use crate::studio::{SideTab, Studio};

const ADD_SOLIDS: [&str; 7] = ["Box", "Cylinder", "Cone", "Sphere", "Torus", "Extrude", "Revolve"];
const ADD_MODIFIERS: [&str; 9] = [
    "Fillet",
    "Chamfer",
    "Shell",
    "Translate",
    "Rotate",
    "Mirror",
    "LinearPattern",
    "PolarPattern",
    "Script",
];

const ERROR: Color32 = Color32::from_rgb(235, 90, 80);

pub fn draw(
    mut contexts: EguiContexts,
    mut studio: ResMut<Studio>,
    window: Single<&Window, With<PrimaryWindow>>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let studio = &mut *studio;
    shortcuts(&ctx, studio);
    let mut root = Ui::new(
        ctx.clone(),
        "chadcad".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );
    let tr = studio.tr.clone();

    egui::Panel::top("menu").show(&mut root, |ui| menu_bar(ui, studio, &tr));
    egui::Panel::top("toolbar").show(&mut root, |ui| toolbar(ui, studio, &tr));
    egui::Panel::bottom("status").show(&mut root, |ui| status_bar(ui, studio, &tr));
    egui::Panel::bottom("timeline").exact_size(84.).show(&mut root, |ui| timeline(ui, studio, &tr));
    egui::Panel::left("left").resizable(true).default_size(380.).show(&mut root, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            parameters(ui, studio, &tr);
            ui.separator();
            editor::feature_panel(ui, studio, &tr);
        });
    });
    egui::Panel::right("right").resizable(true).default_size(330.).show(&mut root, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut studio.tab, SideTab::Ai, tr.t("tab-ai"));
            ui.selectable_value(&mut studio.tab, SideTab::Lua, tr.t("tab-lua"));
        });
        ui.separator();
        match studio.tab {
            SideTab::Ai => ai_panel(ui, studio, &tr),
            SideTab::Lua => lua_panel(ui, studio, &tr),
        }
    });

    let free = root.available_rect_before_wrap();
    let scale = window.scale_factor();
    studio.viewport =
        Rect::new(free.min.x * scale, free.min.y * scale, free.max.x * scale, free.max.y * scale);
    studio.pointer_on_ui = ctx.egui_wants_pointer_input();
    Ok(())
}

fn shortcuts(ctx: &egui::Context, studio: &mut Studio) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let pressed = |modifiers, key| {
        ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
    };
    // Text fields have their own undo.
    if !ctx.egui_wants_keyboard_input() {
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) {
            studio.redo();
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            studio.undo();
        } else if pressed(Modifiers::COMMAND, Key::Y) {
            studio.redo();
        }
    }
    if pressed(Modifiers::COMMAND, Key::S) {
        save(studio);
    }
}

fn save(studio: &mut Studio) {
    match studio.path.clone() {
        Some(path) => studio.save_to(path),
        None => save_as(studio),
    }
}

fn save_as(studio: &mut Studio) {
    let dialog = rfd::FileDialog::new()
        .add_filter("ChadCAD", &[chadcad_model::FILE_EXTENSION])
        .set_file_name(format!("{}.{}", studio.doc.name, chadcad_model::FILE_EXTENSION));
    if let Some(path) = dialog.save_file() {
        studio.save_to(path);
    }
}

fn menu_bar(ui: &mut Ui, studio: &mut Studio, tr: &Arc<I18n>) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(tr.t("menu-file"), |ui| {
            if ui.button(tr.t("menu-new")).clicked() {
                studio.new_document();
                ui.close();
            }
            if ui.button(tr.t("menu-open")).clicked() {
                let filter = [chadcad_model::FILE_EXTENSION];
                if let Some(path) =
                    rfd::FileDialog::new().add_filter("ChadCAD", &filter).pick_file()
                {
                    studio.open_path(path);
                }
                ui.close();
            }
            if ui.button(tr.t("menu-save")).clicked() {
                save(studio);
                ui.close();
            }
            if ui.button(tr.t("menu-save-as")).clicked() {
                save_as(studio);
                ui.close();
            }
            ui.separator();
            ui.menu_button(tr.t("menu-export"), |ui| {
                for format in Format::ALL {
                    if ui.button(format.label()).clicked() {
                        let dialog = rfd::FileDialog::new()
                            .add_filter(format.label(), &[format.extension()])
                            .set_file_name(format!("{}.{}", studio.doc.name, format.extension()));
                        if let Some(path) = dialog.save_file() {
                            studio.export(path, format);
                        }
                        ui.close();
                    }
                }
            });
        });
        ui.menu_button(tr.t("menu-edit"), |ui| {
            if ui.add_enabled(studio.can_undo(), egui::Button::new(tr.t("menu-undo"))).clicked() {
                studio.undo();
                ui.close();
            }
            if ui.add_enabled(studio.can_redo(), egui::Button::new(tr.t("menu-redo"))).clicked() {
                studio.redo();
                ui.close();
            }
        });
        ui.menu_button(tr.t("menu-view"), |ui| {
            if ui.button(tr.t("menu-fit")).clicked() {
                studio.fit_requested = true;
                ui.close();
            }
        });
        ui.menu_button(tr.t("menu-language"), |ui| {
            for (code, name, _) in LANGUAGES {
                if ui.radio(tr.code() == code, name).clicked() {
                    studio.tr = Arc::new(I18n::new(code));
                    ui.close();
                }
            }
        });
    });
}

fn toolbar(ui: &mut Ui, studio: &mut Studio, tr: &Arc<I18n>) {
    let mut add = None;
    ui.horizontal_wrapped(|ui| {
        for tag in ADD_SOLIDS {
            if ui.button(tr.t(&format!("feature-{tag}"))).clicked() {
                add = Some(tag);
            }
        }
        ui.separator();
        for tag in ADD_MODIFIERS {
            if ui.button(tr.t(&format!("feature-{tag}"))).clicked() {
                add = Some(tag);
            }
        }
    });
    if let Some(tag) = add
        && let Some(kind) = editor::default_feature(tag)
    {
        let mut new_id = None;
        studio.edit(|doc| new_id = Some(doc.add_feature(kind)));
        studio.select(new_id);
    }
}

fn parameters(ui: &mut Ui, studio: &mut Studio, tr: &Arc<I18n>) {
    ui.heading(tr.t("panel-parameters"));
    let mut commit = None;
    let mut remove = None;
    egui::Grid::new("parameters").num_columns(4).striped(true).show(ui, |ui| {
        for (i, (name, expr)) in studio.param_drafts.iter_mut().enumerate() {
            ui.label(name.as_str());
            let edit = ui.add_sized([130., 20.], TextEdit::singleline(expr));
            if edit.lost_focus() {
                commit = Some(i);
            }
            match studio.build.parameters.iter().find(|(n, _)| n == name) {
                Some((_, Ok(value))) => {
                    ui.label(format!("{value:.4}"));
                }
                Some((_, Err(error))) => {
                    ui.colored_label(ERROR, "!").on_hover_text(error);
                }
                None => {
                    ui.label("");
                }
            }
            if ui.small_button("x").on_hover_text(tr.t("action-delete")).clicked() {
                remove = Some(name.clone());
            }
            ui.end_row();
        }
    });
    if let Some(i) = commit {
        let (name, expr) = studio.param_drafts[i].clone();
        studio.edit(|doc| doc.set_parameter(&name, Expr(expr)));
    }
    if let Some(name) = remove {
        studio.edit(|doc| doc.parameters.retain(|p| p.name != name));
    }
    ui.horizontal(|ui| {
        let hint = tr.t("param-new-hint");
        ui.add(TextEdit::singleline(&mut studio.new_param).hint_text(hint).desired_width(120.));
        let name = studio.new_param.trim().to_owned();
        let valid = !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !studio.doc.parameters.iter().any(|p| p.name == name);
        if ui.add_enabled(valid, egui::Button::new(tr.t("action-add"))).clicked() {
            studio.edit(|doc| doc.set_parameter(&name, "0".into()));
            studio.new_param.clear();
        }
    });
}

fn timeline(ui: &mut Ui, studio: &mut Studio, tr: &Arc<I18n>) {
    let total = studio.doc.features.len();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr.t("panel-timeline")).strong());
        let mut position = studio.doc.rollback.unwrap_or(total).min(total);
        let slider = egui::Slider::new(&mut position, 0..=total).text(tr.t("timeline-rollback"));
        if ui.add_enabled(total > 0, slider).changed() {
            let rollback = (position < total).then_some(position);
            studio.edit(|doc| doc.rollback = rollback);
        }
    });
    enum Action {
        Select(u64),
        Toggle(u64),
        Move(u64, isize),
        Delete(u64),
        Roll(usize),
    }
    let mut action = None;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            let active = studio.doc.rollback.unwrap_or(total);
            for (i, feature) in studio.doc.features.iter().enumerate() {
                let failed = studio.build.errors.get(&feature.id);
                let mut text = RichText::new(&feature.name);
                if feature.suppressed {
                    text = text.strikethrough().weak();
                }
                if i >= active {
                    text = text.weak();
                }
                if failed.is_some() {
                    text = text.color(ERROR);
                }
                let chip = ui.selectable_label(studio.selected == Some(feature.id), text);
                let chip = chip.on_hover_ui(|ui| {
                    ui.label(tr.t(&format!("feature-{}", feature.kind.tag())));
                    if let Some(error) = failed {
                        ui.colored_label(ERROR, error);
                    }
                });
                if chip.clicked() {
                    action = Some(Action::Select(feature.id));
                }
                chip.context_menu(|ui| {
                    let label =
                        if feature.suppressed { "action-unsuppress" } else { "action-suppress" };
                    if ui.button(tr.t(label)).clicked() {
                        action = Some(Action::Toggle(feature.id));
                        ui.close();
                    }
                    if ui.button(tr.t("action-move-earlier")).clicked() {
                        action = Some(Action::Move(feature.id, -1));
                        ui.close();
                    }
                    if ui.button(tr.t("action-move-later")).clicked() {
                        action = Some(Action::Move(feature.id, 1));
                        ui.close();
                    }
                    if ui.button(tr.t("action-roll-here")).clicked() {
                        action = Some(Action::Roll(i + 1));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(tr.t("action-delete")).clicked() {
                        action = Some(Action::Delete(feature.id));
                        ui.close();
                    }
                });
                ui.label(RichText::new("›").weak());
            }
        });
    });
    match action {
        Some(Action::Select(id)) => studio.select(Some(id)),
        Some(Action::Toggle(id)) => studio.edit(|doc| {
            if let Some(f) = doc.feature_mut(id) {
                f.suppressed = !f.suppressed;
            }
        }),
        Some(Action::Move(id, by)) => studio.edit(|doc| doc.move_feature(id, by)),
        Some(Action::Delete(id)) => studio.edit(|doc| {
            doc.remove_feature(id);
        }),
        Some(Action::Roll(to)) => studio.edit(|doc| {
            doc.rollback = (to < doc.features.len()).then_some(to);
        }),
        None => {}
    }
}

fn status_bar(ui: &mut Ui, studio: &Studio, tr: &Arc<I18n>) {
    ui.horizontal(|ui| {
        let name = &studio.doc.name;
        ui.label(if studio.dirty { format!("{name} *") } else { name.clone() });
        ui.separator();
        ui.label(&studio.status);
        ui.separator();
        if studio.building {
            ui.spinner();
            ui.label(tr.t("status-building"));
        } else {
            let count = studio.build.rebuilt.to_string();
            ui.label(tr.t_with("status-rebuilt", &[("count", count)]));
        }
        if let Some(p) = studio.build.properties {
            ui.separator();
            let size: [f64; 3] = std::array::from_fn(|i| p.bbox_max[i] - p.bbox_min[i]);
            let size = format!("{:.2} × {:.2} × {:.2}", size[0], size[1], size[2]);
            ui.label(tr.t_with(
                "status-properties",
                &[
                    ("volume", format!("{:.2}", p.volume)),
                    ("area", format!("{:.2}", p.area)),
                    ("size", size),
                ],
            ));
        }
        if !studio.build.errors.is_empty() {
            ui.separator();
            let count = studio.build.errors.len().to_string();
            ui.colored_label(ERROR, tr.t_with("status-errors", &[("count", count)]));
        }
    });
}

fn ai_panel(ui: &mut Ui, studio: &mut Studio, tr: &Arc<I18n>) {
    ui.horizontal(|ui| {
        ui.label(tr.t("ai-model"));
        ui.add(TextEdit::singleline(&mut studio.ai.model).desired_width(f32::INFINITY));
    });
    let height = (ui.available_height() - 110.).max(60.);
    egui::ScrollArea::vertical().max_height(height).stick_to_bottom(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if studio.ai.lines.is_empty() {
            ui.label(RichText::new(tr.t("ai-hint")).weak());
        }
        for line in &studio.ai.lines {
            if line.from_user {
                ui.label(RichText::new(&line.text).strong());
            } else {
                ui.label(&line.text);
            }
            ui.add_space(6.);
        }
    });
    ui.separator();
    let input = ui.add(
        TextEdit::multiline(&mut studio.ai.input)
            .hint_text(tr.t("ai-prompt"))
            .desired_rows(3)
            .desired_width(f32::INFINITY),
    );
    let enter = input.has_focus()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
    ui.horizontal(|ui| {
        let running = studio.ai_running();
        let send = ui.add_enabled(!running, egui::Button::new(tr.t("ai-send"))).clicked();
        if running {
            ui.spinner();
        }
        if send || enter {
            studio.send_to_ai();
        }
    });
}

fn lua_panel(ui: &mut Ui, studio: &mut Studio, tr: &Arc<I18n>) {
    let source = TextEdit::multiline(&mut studio.lua_source)
        .code_editor()
        .desired_rows(12)
        .desired_width(f32::INFINITY);
    ui.add(source);
    if ui.button(tr.t("lua-run")).clicked() {
        studio.run_lua();
    }
    ui.separator();
    egui::ScrollArea::vertical().id_salt("lua").max_height(160.).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.monospace(&studio.lua_output);
    });
    ui.collapsing(tr.t("lua-reference"), |ui| {
        egui::ScrollArea::vertical().id_salt("reference").show(ui, |ui| {
            ui.monospace(chadcad_lua::API_REFERENCE);
        });
    });
}

/// The label for a feature kind's i18n key, for tests.
#[cfg(test)]
pub fn feature_tags() -> impl Iterator<Item = &'static str> {
    ADD_SOLIDS.into_iter().chain(ADD_MODIFIERS)
}
