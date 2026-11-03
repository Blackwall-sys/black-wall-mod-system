// BWMS — `tweakxl-script-extensions` (2026-07-26): ScriptableTweak nativo.
// BWMS instancia cada subclasse concreta e chama OnApply() pós-Session/Start.
// Fonte real: enablers/TweakXL/scripts/ScriptableTweak.reds

public abstract native class ScriptableTweak {
  protected cb func OnApply() -> Void {}
}
