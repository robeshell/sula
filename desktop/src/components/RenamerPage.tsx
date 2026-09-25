import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from "./ui/table";
import { Checkbox } from "./ui/checkbox";
import { NativeSelect } from "./ui/native-select";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import { WindowControls } from "./WindowControls";
import { isImmersiveWindow } from "../lib/windowChrome";

type RuleType =
  | "textReplace"
  | "regexReplace"
  | "insertText"
  | "deleteRange"
  | "caseConversion"
  | "autoNumbering"
  | "stripBrackets";

type AnyRenameRule = {
  type: RuleType;
  id: string;
  find?: string;
  replacement?: string;
  pattern?: string;
  text?: string;
  position?: number | "prefix" | "suffix";
  from?: number;
  length?: number;
  mode?: "title" | "lower" | "upper";
  startAt?: number;
  padding?: number;
  separator?: string;
  bracketTypes?: Array<"square" | "round" | "curly">;
};

type FileEntry = {
  id: string;
  originalName: string;
  path: string;
};

type PreviewResult = {
  id: string;
  originalName: string;
  newName: string;
  path: string;
  hasConflict: boolean;
  hasInvalidChars: boolean;
};

function newId() {
  return crypto.randomUUID();
}

function defaultRule(type: RuleType): AnyRenameRule {
  switch (type) {
    case "textReplace":
      return { type, id: newId(), find: "", replacement: "" };
    case "regexReplace":
      return { type, id: newId(), pattern: "", replacement: "" };
    case "insertText":
      return { type, id: newId(), text: "", position: 0 };
    case "deleteRange":
      return { type, id: newId(), from: 0, length: 1 };
    case "caseConversion":
      return { type, id: newId(), mode: "title" };
    case "autoNumbering":
      return {
        type,
        id: newId(),
        startAt: 1,
        padding: 2,
        position: "prefix",
        separator: " ",
      };
    case "stripBrackets":
      return { type, id: newId(), bracketTypes: ["square", "round"] };
  }
}

function isExecutable(p: PreviewResult) {
  return (
    !p.hasConflict &&
    !p.hasInvalidChars &&
    p.originalName !== p.newName &&
    p.newName.length > 0
  );
}

export function RenamerPage() {
  const { t } = useTranslation();
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [rules, setRules] = useState<AnyRenameRule[]>([defaultRule("textReplace")]);
  const [previews, setPreviews] = useState<PreviewResult[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [snapshotCount, setSnapshotCount] = useState(0);
  const [addType, setAddType] = useState<RuleType>("textReplace");
  const [presets, setPresets] = useState<string[]>([]);
  const [selectedPreset, setSelectedPreset] = useState("");
  const [presetName, setPresetName] = useState("");
  const [rulesReady, setRulesReady] = useState(false);

  const executableCount = useMemo(
    () => previews.filter(isExecutable).length,
    [previews],
  );

  useEffect(() => {
    void (async () => {
      try {
        const [count, names, auto] = await Promise.all([
          invoke<number>("renamer_snapshot_count"),
          invoke<string[]>("renamer_list_presets"),
          invoke<{ rules: AnyRenameRule[] } | null>("renamer_auto_load_pipeline"),
        ]);
        setSnapshotCount(count);
        setPresets(names);
        if (auto?.rules?.length) {
          setRules(auto.rules);
        }
      } catch {
        setSnapshotCount(0);
      } finally {
        setRulesReady(true);
      }
    })();
  }, []);

  useEffect(() => {
    if (!rulesReady) return;
    const timer = window.setTimeout(() => {
      void invoke("renamer_auto_save_pipeline", { pipeline: { rules } }).catch(() => {});
    }, 400);
    return () => window.clearTimeout(timer);
  }, [rules, rulesReady]);

  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        if (files.length === 0) {
          if (!cancelled) setPreviews([]);
          return;
        }
        try {
          const out = await invoke<PreviewResult[]>("renamer_preview", {
            files,
            pipeline: { rules },
          });
          if (!cancelled) setPreviews(out);
        } catch (err) {
          if (!cancelled) setMessage(String(err));
        }
      })();
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [files, rules]);

  async function addPaths(directory: boolean) {
    setMessage(null);
    const selected = await open({
      directory,
      multiple: !directory,
      title: directory ? t("renamer.pickFolder") : t("renamer.pickFiles"),
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    try {
      setBusy(true);
      const collected = await invoke<FileEntry[]>("renamer_collect_files", { paths });
      setFiles((prev) => {
        const seen = new Set(prev.map((f) => f.path));
        const merged = [...prev];
        for (const f of collected) {
          if (!seen.has(f.path)) {
            seen.add(f.path);
            merged.push(f);
          }
        }
        return merged;
      });
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function runExecute() {
    setMessage(null);
    try {
      setBusy(true);
      const done = await invoke<{ originalPath: string; newPath: string }[]>(
        "renamer_execute",
        { previews },
      );
      const count = await invoke<number>("renamer_snapshot_count");
      setSnapshotCount(count);
      setMessage(t("renamer.executed", { count: done.length }));
      // Refresh entries from new paths.
      const nextPaths = previews.map((p) => {
        if (!isExecutable(p)) return p.path;
        const parent = p.path.replace(/[/\\][^/\\]+$/, "");
        const sep = p.path.includes("\\") ? "\\" : "/";
        return `${parent}${sep}${p.newName}`;
      });
      const refreshed = await invoke<FileEntry[]>("renamer_collect_files", {
        paths: nextPaths,
      });
      setFiles(refreshed);
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function runUndo() {
    setMessage(null);
    try {
      setBusy(true);
      const n = await invoke<number>("renamer_undo_last");
      const count = await invoke<number>("renamer_snapshot_count");
      setSnapshotCount(count);
      setMessage(t("renamer.undone", { count: n }));
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  function updateRule(id: string, patch: Partial<AnyRenameRule>) {
    setRules((prev) => prev.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  }

  async function refreshPresets() {
    const names = await invoke<string[]>("renamer_list_presets");
    setPresets(names);
  }

  async function loadPreset(name: string) {
    if (!name) return;
    setMessage(null);
    try {
      const pipeline = await invoke<{ rules: AnyRenameRule[] } | null>("renamer_load_preset", {
        name,
      });
      if (pipeline?.rules) {
        setRules(pipeline.rules);
        setSelectedPreset(name);
        setMessage(t("renamer.preset.loaded", { name }));
      }
    } catch (err) {
      setMessage(String(err));
    }
  }

  async function savePreset() {
    const name = (presetName.trim() || selectedPreset).trim();
    if (!name) {
      setMessage(t("renamer.preset.nameRequired"));
      return;
    }
    setMessage(null);
    try {
      await invoke("renamer_save_preset", { name, pipeline: { rules } });
      await refreshPresets();
      setSelectedPreset(name);
      setPresetName("");
      setMessage(t("renamer.preset.saved", { name }));
    } catch (err) {
      setMessage(String(err));
    }
  }

  async function deletePreset() {
    const name = selectedPreset;
    if (!name) return;
    setMessage(null);
    try {
      await invoke("renamer_delete_preset", { name });
      await refreshPresets();
      setSelectedPreset("");
      setMessage(t("renamer.preset.deleted", { name }));
    } catch (err) {
      setMessage(String(err));
    }
  }

  const immersive = isImmersiveWindow();

  return (
    <div className="kg-shell flex min-h-0 flex-1 flex-col overflow-hidden text-fg">
      {immersive ? (
        <div
          className="kg-titlebar absolute inset-x-0 top-0 z-30 h-[var(--kg-titlebar-height)]"
          aria-hidden
        >
          <div data-tauri-drag-region className="absolute inset-0" />
          <WindowControls />
        </div>
      ) : null}
      <header
        data-tauri-drag-region={!immersive ? true : undefined}
        className={
          immersive
            ? "flex shrink-0 items-center justify-between gap-3 border-b border-hairline px-5 pb-3 pt-[calc(var(--kg-titlebar-height)+0.5rem)] pr-[calc(1.25rem+var(--kg-caption-width))]"
            : "flex shrink-0 items-center justify-between gap-3 border-b border-hairline px-5 pb-3 pt-4"
        }
      >
        <div className="min-w-0">
          <h1 className="kg-page-header-title">{t("renamer.title")}</h1>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <Button variant="ghost" size="sm"
            type="button"
            disabled={busy || snapshotCount === 0}
            onClick={() => void runUndo()}
          >
            {t("renamer.undo")} ({snapshotCount})
          </Button>
          <Button variant="default" size="sm"
            type="button"
            disabled={busy || executableCount === 0}
            onClick={() => void runExecute()}
          >
            {t("renamer.execute")} ({executableCount})
          </Button>
        </div>
      </header>

      {message ? (
        <p className="shrink-0 border-b border-hairline bg-elevated px-5 py-2 kg-type-body-secondary text-fg-secondary">
          {message}
        </p>
      ) : null}

      <div className="grid min-h-0 flex-1 grid-cols-1 gap-0 lg:grid-cols-[minmax(280px,360px)_minmax(0,1fr)]">
        <aside className="flex min-h-0 flex-col border-b border-hairline bg-surface lg:border-b-0 lg:border-r">
          <div className="flex shrink-0 items-center gap-2 px-4 py-3">
            <Button variant="ghost" size="sm"
              type="button"
              disabled={busy}
              onClick={() => void addPaths(false)}
            >
              {t("renamer.addFiles")}
            </Button>
            <Button variant="ghost" size="sm"
              type="button"
              disabled={busy}
              onClick={() => void addPaths(true)}
            >
              {t("renamer.addFolder")}
            </Button>
            <Button variant="ghost" size="sm"
              type="button"
              disabled={files.length === 0}
              onClick={() => setFiles([])}
            >
              {t("renamer.clearFiles")}
            </Button>
          </div>
          <p className="kg-section-label px-4">{t("renamer.files", { count: files.length })}</p>
          <ul className="min-h-0 flex-1 overflow-auto px-2 pb-4">
            {files.length === 0 ? (
              <li className="px-2 py-6 text-center kg-type-body-secondary text-fg-muted">
                {t("renamer.filesEmpty")}
              </li>
            ) : (
              files.map((f) => (
                <li
                  key={f.id}
                  className="truncate rounded-control px-2 py-1.5 kg-type-body-secondary text-fg-secondary"
                  title={f.path}
                >
                  {f.originalName}
                </li>
              ))
            )}
          </ul>

          <div className="shrink-0 space-y-4 border-t border-hairline px-4 py-3">
            <div>
              <p className="kg-section-label">{t("renamer.presets")}</p>
              <div className="mb-2 flex flex-wrap gap-2">
                <NativeSelect
                  className="min-w-0 flex-1"
                  value={selectedPreset}
                  onChange={(e) => void loadPreset(e.target.value)}
                >
                  <option value="">{t("renamer.preset.pick")}</option>
                  {presets.map((name) => (
                    <option key={name} value={name}>
                      {name}
                    </option>
                  ))}
                </NativeSelect>
                <Button variant="ghost" size="sm"
                  type="button"
                  disabled={!selectedPreset}
                  onClick={() => void deletePreset()}
                >
                  {t("common.remove")}
                </Button>
              </div>
              <div className="flex gap-2">
                <Input
                  className="min-w-0 flex-1"
                  placeholder={t("renamer.preset.namePlaceholder")}
                  value={presetName}
                  onChange={(e) => setPresetName(e.target.value)}
                />
                <Button variant="ghost" size="sm"
                  type="button"
                  onClick={() => void savePreset()}
                >
                  {t("renamer.preset.save")}
                </Button>
              </div>
            </div>

            <div>
              <p className="kg-section-label">{t("renamer.rules")}</p>
              <div className="mb-2 flex gap-2">
                <NativeSelect
                  className="min-w-0 flex-1"
                  value={addType}
                  onChange={(e) => setAddType(e.target.value as RuleType)}
                >
                  <option value="textReplace">{t("renamer.rule.textReplace")}</option>
                  <option value="regexReplace">{t("renamer.rule.regexReplace")}</option>
                  <option value="insertText">{t("renamer.rule.insertText")}</option>
                  <option value="deleteRange">{t("renamer.rule.deleteRange")}</option>
                  <option value="caseConversion">{t("renamer.rule.caseConversion")}</option>
                  <option value="autoNumbering">{t("renamer.rule.autoNumbering")}</option>
                  <option value="stripBrackets">{t("renamer.rule.stripBrackets")}</option>
                </NativeSelect>
                <Button variant="ghost" size="sm"
                  type="button"
                  onClick={() => setRules((prev) => [...prev, defaultRule(addType)])}
                >
                  {t("common.add")}
                </Button>
              </div>
              <div className="max-h-[40vh] space-y-2 overflow-auto">
                {rules.map((rule, index) => (
                  <RuleEditor
                    key={rule.id}
                    index={index}
                    rule={rule}
                    onChange={(patch) => updateRule(rule.id, patch)}
                    onRemove={() => setRules((prev) => prev.filter((r) => r.id !== rule.id))}
                  />
                ))}
              </div>
            </div>
          </div>
        </aside>

        <main className="flex min-h-0 flex-col bg-surface">
          <p className="kg-section-label px-4 pt-3">{t("renamer.preview")}</p>
          <div className="min-h-0 flex-1 overflow-auto px-3 pb-4">
            <div className="kg-settings-group overflow-hidden">
              <Table className="w-full border-collapse text-left kg-type-body-secondary">
                <TableHeader className="sticky top-0 bg-[var(--kg-group-fill)] text-fg-muted">
                  <TableRow>
                    <TableHead className="px-3.5 py-2.5 font-semibold">{t("renamer.col.original")}</TableHead>
                    <TableHead className="px-3.5 py-2.5 font-semibold">{t("renamer.col.new")}</TableHead>
                    <TableHead className="px-3.5 py-2.5 font-semibold">{t("renamer.col.status")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {previews.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={3} className="px-3.5 py-8 text-center text-fg-muted">
                        {t("renamer.previewEmpty")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    previews.map((p) => {
                      let status = t("renamer.status.ok");
                      if (p.hasConflict) status = t("renamer.status.conflict");
                      else if (p.hasInvalidChars) status = t("renamer.status.invalid");
                      else if (p.originalName === p.newName) status = t("renamer.status.unchanged");
                      const bad = p.hasConflict || p.hasInvalidChars;
                      return (
                        <TableRow key={p.id} className="border-t border-hairline">
                          <TableCell className="max-w-[240px] truncate px-3.5 py-2 text-fg-secondary">
                            {p.originalName}
                          </TableCell>
                          <TableCell className="max-w-[280px] truncate px-3.5 py-2 font-medium">{p.newName}</TableCell>
                          <TableCell
                            className={`whitespace-nowrap px-3.5 py-2 ${
                              bad ? "text-error" : "text-fg-muted"
                            }`}
                          >
                            {status}
                          </TableCell>
                        </TableRow>
                      );
                    })
                  )}
                </TableBody>
              </Table>
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}

function RuleEditor({
  index,
  rule,
  onChange,
  onRemove,
}: {
  index: number;
  rule: AnyRenameRule;
  onChange: (patch: Partial<AnyRenameRule>) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="rounded-card border border-hairline bg-[var(--kg-group-fill)] px-3 py-2.5">
      <div className="mb-2 flex items-center justify-between gap-2">
        <span className="kg-type-body-secondary font-semibold text-fg-secondary">
          #{index + 1} {t(`renamer.rule.${rule.type}`)}
        </span>
        <Button variant="ghost" size="sm" type="button" onClick={onRemove}>
          {t("common.remove")}
        </Button>
      </div>
      {(rule.type === "textReplace" || rule.type === "regexReplace") && (
        <div className="grid gap-1.5">
          <Input
            placeholder={rule.type === "regexReplace" ? t("renamer.field.pattern") : t("renamer.field.find")}
            value={rule.type === "regexReplace" ? (rule.pattern ?? "") : (rule.find ?? "")}
            onChange={(e) =>
              onChange(
                rule.type === "regexReplace"
                  ? { pattern: e.target.value }
                  : { find: e.target.value },
              )
            }
          />
          <Input
            placeholder={t("renamer.field.replacement")}
            value={rule.replacement ?? ""}
            onChange={(e) => onChange({ replacement: e.target.value })}
          />
        </div>
      )}
      {rule.type === "insertText" && (
        <div className="grid gap-1.5">
          <Input
            placeholder={t("renamer.field.text")}
            value={rule.text ?? ""}
            onChange={(e) => onChange({ text: e.target.value })}
          />
          <label className="flex items-center gap-2 kg-type-caption text-fg-secondary">
            {t("renamer.field.position")}
            <Input
              className="w-20"
              type="number"
              value={typeof rule.position === "number" ? rule.position : 0}
              onChange={(e) => onChange({ position: Number(e.target.value) || 0 })}
            />
          </label>
        </div>
      )}
      {rule.type === "deleteRange" && (
        <div className="flex gap-2">
          <label className="flex flex-1 items-center gap-1 kg-type-caption text-fg-secondary">
            {t("renamer.field.from")}
            <Input
              type="number"
              value={rule.from ?? 0}
              onChange={(e) => onChange({ from: Number(e.target.value) || 0 })}
            />
          </label>
          <label className="flex flex-1 items-center gap-1 kg-type-caption text-fg-secondary">
            {t("renamer.field.length")}
            <Input
              type="number"
              value={rule.length ?? 1}
              onChange={(e) => onChange({ length: Number(e.target.value) || 0 })}
            />
          </label>
        </div>
      )}
      {rule.type === "caseConversion" && (
        <NativeSelect
          className="w-full"
          value={rule.mode ?? "title"}
          onChange={(e) =>
            onChange({ mode: e.target.value as "title" | "lower" | "upper" })
          }
        >
          <option value="title">{t("renamer.case.title")}</option>
          <option value="lower">{t("renamer.case.lower")}</option>
          <option value="upper">{t("renamer.case.upper")}</option>
        </NativeSelect>
      )}
      {rule.type === "autoNumbering" && (
        <div className="grid grid-cols-2 gap-1.5">
          <label className="flex items-center gap-1 kg-type-caption text-fg-secondary">
            {t("renamer.field.startAt")}
            <Input
              type="number"
              value={rule.startAt ?? 1}
              onChange={(e) => onChange({ startAt: Number(e.target.value) || 0 })}
            />
          </label>
          <label className="flex items-center gap-1 kg-type-caption text-fg-secondary">
            {t("renamer.field.padding")}
            <Input
              type="number"
              value={rule.padding ?? 2}
              onChange={(e) => onChange({ padding: Number(e.target.value) || 0 })}
            />
          </label>
          <NativeSelect
            value={rule.position === "suffix" ? "suffix" : "prefix"}
            onChange={(e) => onChange({ position: e.target.value as "prefix" | "suffix" })}
          >
            <option value="prefix">{t("renamer.pos.prefix")}</option>
            <option value="suffix">{t("renamer.pos.suffix")}</option>
          </NativeSelect>
          <Input
            placeholder={t("renamer.field.separator")}
            value={rule.separator ?? " "}
            onChange={(e) => onChange({ separator: e.target.value })}
          />
        </div>
      )}
      {rule.type === "stripBrackets" && (
        <div className="flex flex-wrap gap-3 kg-type-caption text-fg-secondary">
          {(["square", "round", "curly"] as const).map((b) => {
            const checked = (rule.bracketTypes ?? []).includes(b);
            return (
              <label key={b} className="inline-flex items-center gap-1.5">
                <Checkbox

                  checked={checked}
                  onCheckedChange={(e) => {
                    const cur = new Set(rule.bracketTypes ?? []);
                    if (e === true) cur.add(b);
                    else cur.delete(b);
                    onChange({ bracketTypes: Array.from(cur) });
                  }}
                />
                {t(`renamer.bracket.${b}`)}
              </label>
            );
          })}
        </div>
      )}
    </div>
  );
}
