// BWMS — ScriptableTweak nativo (2026-07-26).
// BWMS instancia cada subclasse concreta e chama OnApply() pós-Session/Start.

public abstract native class ScriptableTweak {
  protected cb func OnApply() -> Void {}
}
