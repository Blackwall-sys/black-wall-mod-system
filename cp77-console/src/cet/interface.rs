//! A tabela `ImGui` dos mods CET sobre os widgets do terminal (`terminal::imediato`), sem o ImGui C++ que saiu em 2026-10-03. Cada função
//! desenha na janela de plugin do mod pela `Tela` do quadro (`mods_do_usuario::api::com_tela`), que só existe dentro do `onDraw`; fora
//! dele devolve o valor neutro. Função do ImGui que o terminal não tem devolve `nil` e avisa uma vez no log.

use std::sync::Mutex;

use crate::mods_do_usuario::api::com_tela;
use crate::terminal::imediato::Tela;

/// Barra de abas de um mod: o ImGui declara as abas uma a uma (`BeginTabItem`) e o terminal desenha a barra inteira de uma vez, então a
/// barra deste quadro sai com os rótulos do quadro anterior.
struct BarraDeAbas {
    id: String,
    anteriores: Vec<String>,
    deste_quadro: Vec<String>,
    escolhida: usize,
}

static BARRAS: Mutex<Vec<BarraDeAbas>> = Mutex::new(Vec::new());
/// Barras abertas na janela, a de dentro por último (o JB aninha a "Cameras" numa aba da "Tabbar").
static BARRAS_ABERTAS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `f` na `Tela` do quadro com a janela do mod aberta; `None` fora do `onDraw` ou com a janela recolhida.
fn na_janela<R>(f: impl FnOnce(&mut Tela<'static>) -> R) -> Option<R> {
    com_tela(|t| t.janela_de_plugin_aberta().then(|| f(t))).flatten()
}

fn cor(r: f32, g: f32, b: f32, a: f32) -> [u8; 4] {
    [r, g, b, a].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub(super) fn instalar(lua: &bwms_lua::Lua) -> bwms_lua::Result<()> {
    let ig = lua.create_table()?;
    ig.set("Begin", lua.create_function(|_, (titulo, _resto): (String, bwms_lua::MultiValue)| {
        let aberta = com_tela(|t| t.abrir_janela_de_plugin(&titulo));
        primeira_janela(&titulo, aberta);
        Ok(aberta.unwrap_or(false))
    })?)?;
    ig.set("End", lua.create_function(|_, ()| {
        BARRAS_ABERTAS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        com_tela(|t| t.fechar_janela_de_plugin());
        Ok(())
    })?)?;
    ig.set("Text", lua.create_function(|_, t: String| {
        na_janela(|tela| tela.texto(&t));
        Ok(())
    })?)?;
    ig.set("TextWrapped", lua.create_function(|_, t: String| {
        na_janela(|tela| tela.texto_quebrado(&t));
        Ok(())
    })?)?;
    ig.set("TextColored", lua.create_function(|_, (r, g, b, a, t): (f32, f32, f32, f32, String)| {
        na_janela(|tela| tela.texto_colorido(cor(r, g, b, a), &t));
        Ok(())
    })?)?;
    ig.set("NewLine", lua.create_function(|_, ()| {
        na_janela(|tela| tela.espaco());
        Ok(())
    })?)?;
    ig.set("Spacing", lua.create_function(|_, ()| {
        na_janela(|tela| tela.espaco());
        Ok(())
    })?)?;
    ig.set("Separator", lua.create_function(|_, ()| {
        na_janela(|tela| tela.separador());
        Ok(())
    })?)?;
    ig.set("SameLine", lua.create_function(|_, _: bwms_lua::MultiValue| {
        na_janela(|tela| tela.mesma_linha());
        Ok(())
    })?)?;
    ig.set("Button", lua.create_function(|_, (rotulo, _resto): (String, bwms_lua::MultiValue)| Ok(na_janela(|t| t.botao(&rotulo)).unwrap_or(false)))?)?;
    ig.set("SetTooltip", lua.create_function(|_, t: String| {
        na_janela(|tela| tela.dica(&t));
        Ok(())
    })?)?;
    // Checkbox(rótulo, valor) -> (valor, mudou), como o binding do CET.
    ig.set("Checkbox", lua.create_function(|_, (rotulo, mut v): (String, bool)| {
        let mudou = na_janela(|t| t.caixa_de_marcar(&rotulo, &mut v)).unwrap_or(false);
        Ok((v, mudou))
    })?)?;
    ig.set("SliderInt", lua.create_function(|_, (rotulo, mut v, min, max, _resto): (String, i32, i32, i32, bwms_lua::MultiValue)| {
        let mudou = na_janela(|t| t.deslizante(&rotulo, &mut v, min, max, 0.0)).unwrap_or(false);
        Ok((v, mudou))
    })?)?;
    ig.set("SliderFloat", lua.create_function(|_, (rotulo, mut v, min, max, _resto): (String, f32, f32, f32, bwms_lua::MultiValue)| {
        let mudou = na_janela(|t| t.deslizante_real(&rotulo, &mut v, min, max, 0.0)).unwrap_or(false);
        Ok((v, mudou))
    })?)?;
    ig.set("BeginTabBar", lua.create_function(|_, (id, _resto): (String, bwms_lua::MultiValue)| {
        let mut barras = BARRAS.lock().unwrap_or_else(|e| e.into_inner());
        let i = match barras.iter().position(|b| b.id == id) {
            Some(i) => i,
            None => {
                barras.push(BarraDeAbas { id: id.clone(), anteriores: Vec::new(), deste_quadro: Vec::new(), escolhida: 0 });
                barras.len() - 1
            }
        };
        let b = &mut barras[i];
        // As abas do quadro anterior valem a partir daqui: o ImGui não exige `EndTabBar` (o `End` da janela fecha), e o JB não chama o da barra
        // de fora.
        if !b.deste_quadro.is_empty() {
            b.anteriores = std::mem::take(&mut b.deste_quadro);
        }
        if !b.anteriores.is_empty() {
            let rotulos: Vec<&str> = b.anteriores.iter().map(String::as_str).collect();
            if let Some(e) = na_janela(|t| t.abas(&rotulos)) {
                b.escolhida = e;
            }
        }
        BARRAS_ABERTAS.lock().unwrap_or_else(|e| e.into_inner()).push(id);
        Ok(true)
    })?)?;
    ig.set("BeginTabItem", lua.create_function(|_, (rotulo, _resto): (String, bwms_lua::MultiValue)| {
        let Some(id) = BARRAS_ABERTAS.lock().unwrap_or_else(|e| e.into_inner()).last().cloned() else { return Ok(false) };
        let mut barras = BARRAS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(b) = barras.iter_mut().find(|b| b.id == id) else { return Ok(false) };
        b.deste_quadro.push(rotulo.clone());
        Ok(b.anteriores.get(b.escolhida) == Some(&rotulo))
    })?)?;
    ig.set("EndTabItem", lua.create_function(|_, ()| Ok(()))?)?;
    ig.set("EndTabBar", lua.create_function(|_, ()| {
        BARRAS_ABERTAS.lock().unwrap_or_else(|e| e.into_inner()).pop();
        Ok(())
    })?)?;
    // Layout que o terminal decide sozinho (posição, tamanho, estilo): aceito e sem efeito.
    for nome in ["SetNextWindowPos", "SetNextWindowSize", "SetWindowFontScale", "PushStyleVar", "PopStyleVar", "PushStyleColor", "PopStyleColor",
        "PushItemWidth", "PopItemWidth", "SetNextItemWidth"]
    {
        ig.set(nome, lua.create_function(|_, _: bwms_lua::MultiValue| Ok(()))?)?;
    }
    let meta = lua.create_table()?;
    meta.set(
        "__index",
        lua.create_function(|lua, (_t, nome): (bwms_lua::Table, String)| {
            static AVISADAS: Mutex<Vec<String>> = Mutex::new(Vec::new());
            let mut a = AVISADAS.lock().unwrap_or_else(|e| e.into_inner());
            if !a.contains(&nome) {
                crate::log(&format!("[cet] ImGui.{nome} não existe no terminal do BWMS: a chamada não desenha nada"));
                a.push(nome);
            }
            Ok(bwms_lua::Value::Function(lua.create_function(|_, _: bwms_lua::MultiValue| Ok(bwms_lua::Value::Nil))?))
        })?,
    )?;
    ig.set_metatable(Some(meta));
    lua.globals().set("ImGui", ig)?;
    Ok(())
}

/// `onDraw` dos mods CET no quadro do terminal, com a `Tela` ao alcance da tabela `ImGui`.
pub(crate) fn desenhar(t: &mut Tela<'_>) {
    crate::mods_do_usuario::api::com_tela_dos_plugins(t, || unsafe { super::run_event_draw() });
}

/// 1ª vez que um mod abre cada janela, no canal de veredito: `None` = fora do quadro do terminal (sem `Tela`).
fn primeira_janela(titulo: &str, aberta: Option<bool>) {
    static VISTAS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut v = VISTAS.lock().unwrap_or_else(|e| e.into_inner());
    if !v.iter().any(|t| t == titulo) {
        v.push(titulo.to_string());
        crate::veredito_dev!("[cet] ImGui.Begin \"{titulo}\": aberta={aberta:?}");
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::terminal::imediato::{Alvo, Entrada, Persistente};

    /// A sequência do `onDraw` do JB (janela, barra de abas sem `EndTabBar`, 1ª aba com a caixa, barra aninhada noutra aba) desenha a
    /// "Disable Mod": a barra aprende as abas no 1º quadro e mostra a 1ª a partir do 2º.
    #[test]
    fn caixa_da_1a_aba_do_jb_sai_no_quadro() {
        let lua = bwms_lua::Lua::new();
        instalar(&lua).expect("ImGui");
        let ig: bwms_lua::Table = lua.globals().get("ImGui").expect("tabela");
        let f = |n: &str| -> bwms_lua::Function { ig.get(n).expect(n) };
        let mut p = Persistente::default();
        let mut achou = Vec::new();
        for _ in 0..3 {
            let e = Entrada { procura: Some(Alvo::Rotulo("Disable Mod".into())), ..Entrada::default() };
            let mut t = Tela::nova(e, &mut p, 1);
            crate::mods_do_usuario::api::com_tela_dos_plugins(&mut t, || {
                if f("Begin").call::<bool>("JB Third Person Mod DEBUG MENU").expect("Begin") && f("BeginTabBar").call::<bool>("Tabbar").expect("bar") {
                    if f("BeginTabItem").call::<bool>("Main settings").expect("item") {
                        f("TextColored").call::<()>((0.5, 0.57, 0.59, 1.0, "Fixes")).expect("texto");
                        let _: (bool, bool) = f("Checkbox").call(("Disable Mod", false)).expect("caixa");
                        f("EndTabItem").call::<()>(()).expect("fim item");
                    }
                    if f("BeginTabItem").call::<bool>("Reset camera default").expect("item 2") && f("BeginTabBar").call::<bool>("Cameras").expect("bar 2") {
                        if f("BeginTabItem").call::<bool>("cam 1").expect("cam 1") {
                            f("EndTabItem").call::<()>(()).expect("fim cam 1");
                        }
                        f("EndTabBar").call::<()>(()).expect("fim bar 2");
                        f("EndTabItem").call::<()>(()).expect("fim item 2");
                    }
                    // Como no JB: a barra de fora fica sem `EndTabBar`; o `End` fecha.
                }
                f("End").call::<()>(()).expect("End");
            });
            achou.push(!t.terminar().achados.is_empty());
        }
        assert_eq!(achou, [false, true, true]);
    }
}
