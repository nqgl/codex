const BUNDLE_API_SOURCE: &str = r#"
(() => {
  const invoke = globalThis.__codexBundleInvoke;
  delete globalThis.__codexBundleInvoke;
  const tag = "__codex_bundle_ref";

  const normalizeNames = (names) => {
    if (names.some((name) => typeof name !== "string" || name.length === 0)) {
      throw new TypeError("bundle item names must be non-empty strings");
    }
    return [...new Set(names)];
  };

  const revive = (value) => {
    if (Array.isArray(value)) {
      return value.map(revive);
    }
    if (!value || typeof value !== "object") {
      return value;
    }
    if (Object.prototype.hasOwnProperty.call(value, tag)) {
      return new Bundle(value[tag]);
    }
    for (const key of Object.keys(value)) {
      value[key] = revive(value[key]);
    }
    return value;
  };

  class Bundle {
    constructor(reference) {
      if (!reference || typeof reference.id !== "string" || reference.id.length === 0) {
        throw new TypeError("invalid bundle reference");
      }
      this._reference = Object.freeze({
        id: reference.id,
        selected: Object.freeze([...(reference.selected ?? [])]),
        view: reference.view ?? "full",
      });
    }

    get id() {
      return this._reference.id;
    }

    select(...names) {
      names = normalizeNames(names);
      if (names.length === 0) {
        throw new TypeError("bundle.select expects at least one item name");
      }
      const current = this._reference.selected;
      if (current.length > 0) {
        const missing = names.find((name) => !current.includes(name));
        if (missing !== undefined) {
          throw new RangeError(`bundle item ${JSON.stringify(missing)} is outside the current selection`);
        }
      }
      return new Bundle({...this._reference, selected: names});
    }

    omitted() {
      return new Bundle({...this._reference, view: "omitted"});
    }

    each(...names) {
      const selected = names.length === 0 ? this : this.select(...names);
      return new EachBundle(selected._reference);
    }

    async info() {
      return revive(await invoke({op: "info", bundle: this._reference}));
    }

    async read(item, options = {}) {
      if (typeof item !== "string") {
        options = item ?? {};
        if (this._reference.selected.length !== 1) {
          throw new TypeError("bundle.read requires an item name unless exactly one item is selected");
        }
        [item] = this._reference.selected;
      }
      return revive(await invoke({...options, op: "read", bundle: this._reference, item}));
    }

    async search(query, options = {}) {
      return revive(await invoke({...options, op: "search", bundle: this._reference, query}));
    }

    async ask(question) {
      return revive(await invoke({
        op: "ask",
        bundle: this._reference,
        question,
        each: false,
      }));
    }

    async summarize(instructions) {
      return revive(await invoke({
        op: "summarize",
        bundle: this._reference,
        instructions,
        each: false,
      }));
    }

    toJSON() {
      return {[tag]: this._reference};
    }
  }

  class EachBundle extends Bundle {
    async ask(question) {
      return revive(await invoke({
        op: "ask",
        bundle: this._reference,
        question,
        each: true,
      }));
    }

    async summarize(instructions) {
      return revive(await invoke({
        op: "summarize",
        bundle: this._reference,
        instructions,
        each: true,
      }));
    }
  }

  const rawLoad = globalThis.load;
  globalThis.load = (key) => revive(rawLoad(key));
  globalThis.bundles = Object.freeze({
    create: async (value) => revive(await invoke({op: "create", value})),
    open: (id) => new Bundle({id, selected: [], view: "full"}),
  });
})();
"#;

pub(super) fn install(scope: &mut v8::PinScope<'_, '_>) -> Result<(), String> {
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = tc.init();
    let source = v8::String::new(&tc, BUNDLE_API_SOURCE)
        .ok_or_else(|| "failed to allocate bundle API source".to_string())?;
    let script = v8::Script::compile(&tc, source, None)
        .ok_or_else(|| "failed to compile bundle API".to_string())?;
    script
        .run(&tc)
        .ok_or_else(|| "failed to install bundle API".to_string())?;
    Ok(())
}
