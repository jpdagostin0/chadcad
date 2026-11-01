//! The feature property editor.
//!
//! Every feature kind is edited through its serde form, so a new kind or field
//! appears here without UI code. Field names only pick a widget and a label.

use std::sync::Arc;

use bevy_egui::egui::{self, RichText, TextEdit, Ui};
use chadcad_i18n::I18n;
use chadcad_model::{FeatureId, FeatureKind};
use serde_json::{Map, Value, json};

use crate::studio::Studio;

const OPS: [&str; 3] = ["Join", "Cut", "Intersect"];
const AXES: [&str; 3] = ["X", "Y", "Z"];
const PLANES: [&str; 3] = ["XY", "XZ", "YZ"];
const SIDES: [&str; 6] = ["Top", "Bottom", "Left", "Right", "Front", "Back"];

/// A sensible starting point for each kind, shown in the editor straight away.
pub fn default_feature(tag: &str) -> Option<FeatureKind> {
    let sketch = json!({"plane": "XY", "offset": "0",
        "outer": {"Rect": {"center": ["0", "0"], "size": ["40", "30"]}}, "holes": []});
    let ring = json!({"plane": "XZ", "offset": "0",
        "outer": {"Rect": {"center": ["20", "0"], "size": ["6", "6"]}}, "holes": []});
    let value = match tag {
        "Box" => json!({"Box": {"size": ["40", "30", "10"]}}),
        "Cylinder" => json!({"Cylinder": {"radius": "10", "height": "20"}}),
        "Cone" => json!({"Cone": {"base_radius": "10", "top_radius": "4", "height": "20"}}),
        "Sphere" => json!({"Sphere": {"radius": "10"}}),
        "Torus" => json!({"Torus": {"major": "20", "minor": "4"}}),
        "Extrude" => json!({"Extrude": {"sketch": sketch, "distance": "10"}}),
        "Revolve" => json!({"Revolve": {"sketch": ring}}),
        "Fillet" => json!({"Fillet": {"radius": "1"}}),
        "Chamfer" => json!({"Chamfer": {"distance": "1"}}),
        "Shell" => json!({"Shell": {"open": {"OnSide": "Top"}, "thickness": "2"}}),
        "Translate" => json!({"Translate": {"by": ["10", "0", "0"]}}),
        "Rotate" => json!({"Rotate": {"angle": "45"}}),
        "Mirror" => json!({"Mirror": {"normal": "X", "merge": true}}),
        "LinearPattern" => json!({"LinearPattern": {"count": "3", "spacing": "50"}}),
        "PolarPattern" => json!({"PolarPattern": {"count": "6"}}),
        "Script" => json!({"Script": {"source": "return box(10, 10, 10)"}}),
        _ => return None,
    };
    serde_json::from_value(value).ok()
}

pub fn feature_panel(ui: &mut Ui, studio: &mut Studio, tr: &Arc<I18n>) {
    let Some(id) = studio.selected else {
        ui.label(RichText::new(tr.t("feature-none-selected")).weak());
        return;
    };
    let Some(feature) = studio.doc.feature(id) else { return };
    let tag = feature.kind.tag();
    ui.heading(tr.t(&format!("feature-{tag}")));
    let mut name = feature.name.clone();
    if ui.text_edit_singleline(&mut name).lost_focus() {
        rename(studio, id, name);
    }
    if let Some(error) = studio.build.errors.get(&id) {
        ui.colored_label(egui::Color32::from_rgb(235, 90, 80), error);
    }
    let Some(mut draft) = studio.draft.take() else { return };
    let mut commit = false;
    if let Some(fields) = draft.get_mut(tag).and_then(Value::as_object_mut) {
        object(ui, tr, fields, &mut commit);
    }
    studio.draft = Some(draft.clone());
    if commit {
        match serde_json::from_value::<FeatureKind>(draft) {
            Ok(kind) => studio.edit(|doc| {
                if let Some(feature) = doc.feature_mut(id) {
                    feature.kind = kind;
                }
            }),
            Err(error) => studio.status = error.to_string(),
        }
    }
}

fn rename(studio: &mut Studio, id: FeatureId, name: String) {
    if !name.trim().is_empty() {
        studio.edit(|doc| {
            if let Some(feature) = doc.feature_mut(id) {
                feature.name = name;
            }
        });
    }
}

fn label(tr: &I18n, key: &str) -> String {
    tr.t(&format!("field-{key}"))
}

fn object(ui: &mut Ui, tr: &I18n, fields: &mut Map<String, Value>, commit: &mut bool) {
    for (key, value) in fields.iter_mut() {
        field(ui, tr, key, value, commit);
    }
}

fn choice(
    ui: &mut Ui,
    tr: &I18n,
    key: &str,
    options: &[&str],
    value: &mut Value,
    commit: &mut bool,
) {
    let current = value.as_str().unwrap_or("").to_owned();
    ui.horizontal(|ui| {
        ui.label(label(tr, key));
        egui::ComboBox::from_id_salt((key, ui.id()))
            .selected_text(tr.t(&format!("choice-{current}")))
            .show_ui(ui, |ui| {
                for option in options {
                    let text = tr.t(&format!("choice-{option}"));
                    if ui.selectable_label(current == *option, text).clicked() {
                        *value = json!(option);
                        *commit = true;
                    }
                }
            });
    });
}

fn expression(ui: &mut Ui, value: &mut Value, width: f32, commit: &mut bool) {
    let mut text = value.as_str().unwrap_or("").to_owned();
    let response = ui.add(TextEdit::singleline(&mut text).desired_width(width));
    if response.changed() {
        *value = json!(text);
    }
    *commit |= response.lost_focus();
}

fn tuple(ui: &mut Ui, tr: &I18n, key: &str, items: &mut [Value], commit: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(label(tr, key));
        for item in items {
            expression(ui, item, 84., commit);
        }
    });
}

/// An edge or face selector, as a flat list of what it can be.
fn selector(ui: &mut Ui, tr: &I18n, key: &str, value: &mut Value, commit: &mut bool) {
    let edges = key == "edges";
    let mut options = Vec::new();
    if edges {
        options.push(json!("All"));
        options.push(json!("Curved"));
        options.extend((0..3).map(|axis| json!({"Parallel": axis})));
    }
    options.extend(SIDES.iter().map(|side| json!({"OnSide": side})));
    let describe = |v: &Value| match v {
        Value::String(s) => tr.t(&format!("choice-{s}")),
        Value::Object(o) => {
            if let Some(axis) = o.get("Parallel").and_then(Value::as_u64) {
                let axis = AXES[axis.min(2) as usize].to_owned();
                tr.t_with("choice-parallel", &[("axis", axis)])
            } else if let Some(side) = o.get("OnSide").and_then(Value::as_str) {
                tr.t(&format!("choice-{side}"))
            } else {
                tr.t("choice-nearest")
            }
        }
        _ => String::new(),
    };
    ui.horizontal(|ui| {
        ui.label(label(tr, key));
        let text = describe(value);
        egui::ComboBox::from_id_salt((key, ui.id())).selected_text(text).show_ui(ui, |ui| {
            for option in &options {
                if ui.selectable_label(value == option, describe(option)).clicked() {
                    *value = option.clone();
                    *commit = true;
                }
            }
        });
    });
}

fn default_profile(variant: &str) -> Value {
    match variant {
        "Circle" => json!({"Circle": {"center": ["0", "0"], "radius": "10"}}),
        "Polygon" => json!({"Polygon": {"points": [["0", "0"], ["20", "0"], ["10", "15"]]}}),
        "RegularPolygon" => {
            json!({"RegularPolygon": {"center": ["0", "0"], "radius": "10", "sides": "6"}})
        }
        _ => json!({"Rect": {"center": ["0", "0"], "size": ["20", "20"]}}),
    }
}

fn profile(ui: &mut Ui, tr: &I18n, key: &str, value: &mut Value, commit: &mut bool) {
    const VARIANTS: [&str; 4] = ["Rect", "Circle", "Polygon", "RegularPolygon"];
    let Some(map) = value.as_object() else { return };
    let current = map.keys().next().cloned().unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(label(tr, key));
        egui::ComboBox::from_id_salt(("profile", ui.id()))
            .selected_text(tr.t(&format!("choice-{current}")))
            .show_ui(ui, |ui| {
                for variant in VARIANTS {
                    let text = tr.t(&format!("choice-{variant}"));
                    if ui.selectable_label(current == variant, text).clicked() {
                        *value = default_profile(variant);
                        *commit = true;
                    }
                }
            });
    });
    if let Some(fields) =
        value.as_object_mut().and_then(|m| m.values_mut().next()).and_then(Value::as_object_mut)
    {
        object(ui, tr, fields, commit);
    }
}

fn list(ui: &mut Ui, tr: &I18n, key: &str, items: &mut Vec<Value>, commit: &mut bool) {
    ui.label(label(tr, key));
    let mut remove = None;
    for (i, item) in items.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| match item {
                    Value::Array(pair) => {
                        ui.horizontal(|ui| {
                            for coordinate in pair {
                                expression(ui, coordinate, 84., commit);
                            }
                        });
                    }
                    other => profile(ui, tr, "hole", other, commit),
                });
                if ui.small_button("x").on_hover_text(tr.t("action-delete")).clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        items.remove(i);
        *commit = true;
    }
    if ui.small_button(tr.t("action-add")).clicked() {
        let next = match (key, items.last()) {
            ("holes", _) => json!({"Circle": {"center": ["0", "0"], "radius": "3"}}),
            (_, Some(last)) => last.clone(),
            _ => json!(["0", "0"]),
        };
        items.push(next);
        *commit = true;
    }
}

fn field(ui: &mut Ui, tr: &I18n, key: &str, value: &mut Value, commit: &mut bool) {
    match key {
        "op" => return choice(ui, tr, key, &OPS, value, commit),
        "axis" | "normal" => return choice(ui, tr, key, &AXES, value, commit),
        "plane" => return choice(ui, tr, key, &PLANES, value, commit),
        "edges" | "open" => return selector(ui, tr, key, value, commit),
        "outer" => return profile(ui, tr, key, value, commit),
        "source" => {
            ui.label(label(tr, key));
            let mut text = value.as_str().unwrap_or("").to_owned();
            let response = ui.add(
                TextEdit::multiline(&mut text)
                    .code_editor()
                    .desired_rows(8)
                    .desired_width(f32::INFINITY),
            );
            if response.changed() {
                *value = json!(text);
            }
            *commit |= response.lost_focus();
            return;
        }
        _ => {}
    }
    match value {
        Value::Bool(on) => {
            if ui.checkbox(on, label(tr, key)).changed() {
                *commit = true;
            }
        }
        Value::String(_) => {
            ui.horizontal(|ui| {
                ui.label(label(tr, key));
                expression(ui, value, 110., commit);
            });
        }
        Value::Array(items) if key == "holes" || key == "points" => {
            list(ui, tr, key, items, commit);
        }
        Value::Array(items) => tuple(ui, tr, key, items, commit),
        Value::Object(fields) => {
            ui.group(|ui| {
                ui.label(RichText::new(label(tr, key)).strong());
                object(ui, tr, fields, commit);
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_toolbar_button_has_a_default_and_translations() {
        let english = chadcad_i18n::I18n::new("en");
        for tag in crate::ui::feature_tags() {
            let kind = default_feature(tag).unwrap_or_else(|| panic!("no default for {tag}"));
            assert_eq!(kind.tag(), tag);
            let key = format!("feature-{tag}");
            assert_ne!(english.t(&key), key);
        }
    }

    /// Every field and choice the generic editor can show must have a label.
    #[test]
    fn every_field_and_choice_is_translated() {
        let english = chadcad_i18n::I18n::new("en");
        fn walk(value: &Value, keys: &mut Vec<String>) {
            match value {
                Value::Object(map) => {
                    for (k, v) in map {
                        keys.push(k.clone());
                        walk(v, keys);
                    }
                }
                Value::Array(items) => items.iter().for_each(|v| walk(v, keys)),
                _ => {}
            }
        }
        let mut keys = vec!["hole".to_owned()];
        for tag in crate::ui::feature_tags() {
            let kind = default_feature(tag).unwrap();
            walk(&serde_json::to_value(&kind).unwrap()[tag], &mut keys);
        }
        const VARIANTS: [&str; 4] = ["Rect", "Circle", "Polygon", "RegularPolygon"];
        keys.retain(|k| !VARIANTS.contains(&k.as_str()) && k != "OnSide" && k != "Parallel");
        for key in keys {
            let key = format!("field-{key}");
            assert_ne!(english.t(&key), key);
        }
        let choices = OPS.iter().chain(&AXES).chain(&PLANES).chain(&SIDES).chain(&VARIANTS);
        for choice in choices.chain(&["All", "Curved"]) {
            let key = format!("choice-{choice}");
            assert_ne!(english.t(&key), key);
        }
    }

    /// Every key spelled out in the UI source must exist.
    #[test]
    fn literal_keys_exist() {
        let english = chadcad_i18n::I18n::new("en");
        let sources = [include_str!("ui.rs"), include_str!("editor.rs"), include_str!("studio.rs")];
        for source in sources {
            for (i, _) in source.match_indices("tr.t") {
                let rest = &source[i..];
                let Some(open) = rest.find('"').filter(|o| *o < 14) else { continue };
                let key = rest[open + 1..].split('"').next().unwrap_or("");
                if !key.contains('{') {
                    assert_ne!(english.t(key), key, "untranslated {key}");
                }
            }
        }
    }
}
