"use client";

import {trpc} from "@/trpc/client";
import {useEffect, useRef, useState} from "react";
import {useParams, useRouter} from "next/navigation";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatUptime(seconds: string | number): string {
    const s = typeof seconds === "string" ? parseInt(seconds, 10) : seconds;
    if (isNaN(s)) return "—";
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    return `${h}h ${m}m`;
}

function formatTs(ts: string): string {
    const n = Number(ts);
    if (!n) return "—";
    return new Date(n / 1_000_000).toISOString();
}


function uint128ToUuid(id: string): string {
    try {
        const hex = BigInt(id).toString(16).padStart(32, "0");
        return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
    } catch {
        return id;
    }
}

function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

function IdFormatToggle({format, onToggle}: { format: "uint128" | "uuid"; onToggle: () => void }) {
    return (
        <button
            onClick={(e) => {
                e.stopPropagation();
                onToggle();
            }}
            className="ml-1 rounded border border-gray-300 px-1 py-0.5 text-[10px] font-normal text-gray-400 hover:border-gray-500 hover:text-gray-600"
        >
            {format === "uuid" ? "UUID" : "UInt128"}
        </button>
    );
}

function FlagsFormatToggle({format, onToggle}: { format: "hex" | "bin"; onToggle: () => void }) {
    return (
        <button
            onClick={(e) => {
                e.stopPropagation();
                onToggle();
            }}
            className="ml-1 rounded border border-gray-300 px-1 py-0.5 text-[10px] font-normal text-gray-400 hover:border-gray-500 hover:text-gray-600"
        >
            {format === "hex" ? "0x" : "bin"}
        </button>
    );
}

function CopyButton({value}: { value: string }) {
    const [copied, setCopied] = useState(false);
    return (
        <button
            title="Copy"
            onClick={() => {
                navigator.clipboard.writeText(value).catch(() => {
                });
                setCopied(true);
                setTimeout(() => setCopied(false), 1500);
            }}
            className="ml-1.5 rounded px-1 py-0.5 text-xs text-gray-400 hover:bg-gray-100 hover:text-gray-600"
        >
            {copied ? "✓" : "⎘"}
        </button>
    );
}

// ---------------------------------------------------------------------------
// Process state badge
// ---------------------------------------------------------------------------

function ProcessStateBadge({state}: { state: string }) {
    const colors: Record<string, string> = {
        PROCESS_STATE_RUNNING: "bg-green-100 text-green-800",
        PROCESS_STATE_STOPPED: "bg-gray-100 text-gray-600",
        PROCESS_STATE_CRASHED: "bg-red-100 text-red-800",
        PROCESS_STATE_STARTING: "bg-yellow-100 text-yellow-800",
    };
    const labels: Record<string, string> = {
        PROCESS_STATE_RUNNING: "Running",
        PROCESS_STATE_STOPPED: "Stopped",
        PROCESS_STATE_CRASHED: "Crashed",
        PROCESS_STATE_STARTING: "Starting",
    };
    return (
        <span
            className={`inline-block rounded-full px-2.5 py-0.5 text-xs font-semibold ${colors[state] ?? "bg-gray-100 text-gray-600"}`}>
            {labels[state] ?? "Unknown"}
        </span>
    );
}

// ---------------------------------------------------------------------------
// Backup config editor
// ---------------------------------------------------------------------------

interface BackupConfigForm {
    aws_endpoint_url: string;
    aws_access_key_id: string;
    aws_secret_access_key: string;
    aws_default_region: string;
    aws_request_checksum_calculation: string;
    aws_response_checksum_validation: string;
    bucket: string;
    backup_file: string;
}

const EMPTY_FORM: BackupConfigForm = {
    aws_endpoint_url: "",
    aws_access_key_id: "",
    aws_secret_access_key: "",
    aws_default_region: "",
    aws_request_checksum_calculation: "",
    aws_response_checksum_validation: "",
    bucket: "",
    backup_file: "",
};

function BackupConfigEditor({nodeId}: { nodeId: string }) {
    const [open, setOpen] = useState(false);
    const [form, setForm] = useState<BackupConfigForm>(EMPTY_FORM);
    const [showSecret, setShowSecret] = useState(false);
    const [saveResult, setSaveResult] = useState<{ ok: boolean; msg: string } | null>(null);

    const configQuery = trpc.manager.getBackupConfig.useQuery({nodeId}, {enabled: open, staleTime: 0});
    const modifyMutation = trpc.manager.modifyBackupConfig.useMutation();

    useEffect(() => {
        if (configQuery.data) {
            setForm({
                aws_endpoint_url: configQuery.data.aws_endpoint_url,
                aws_access_key_id: configQuery.data.aws_access_key_id,
                aws_secret_access_key: configQuery.data.aws_secret_access_key,
                aws_default_region: configQuery.data.aws_default_region,
                aws_request_checksum_calculation: configQuery.data.aws_request_checksum_calculation,
                aws_response_checksum_validation: configQuery.data.aws_response_checksum_validation,
                bucket: configQuery.data.bucket,
                backup_file: configQuery.data.backup_file,
            });
        }
    }, [configQuery.data]);

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        setSaveResult(null);
        try {
            const result = await modifyMutation.mutateAsync({
                nodeId,
                awsEndpointUrl: form.aws_endpoint_url,
                awsAccessKeyId: form.aws_access_key_id,
                awsSecretAccessKey: form.aws_secret_access_key,
                awsDefaultRegion: form.aws_default_region,
                awsRequestChecksumCalculation: form.aws_request_checksum_calculation,
                awsResponseChecksumValidation: form.aws_response_checksum_validation,
                bucket: form.bucket,
                backupFile: form.backup_file,
            });
            setSaveResult({ok: result.success, msg: result.message});
            if (result.success) configQuery.refetch();
        } catch (err: unknown) {
            const msg = err instanceof Error ? err.message : "Unknown error";
            setSaveResult({ok: false, msg});
        }
    };

    const field = (label: string, key: keyof BackupConfigForm, opts?: {
        placeholder?: string;
        isSecret?: boolean;
        mono?: boolean;
    }) => (
        <div>
            <label className="mb-0.5 block text-xs font-medium text-gray-500">{label}</label>
            <input
                type={opts?.isSecret && !showSecret ? "password" : "text"}
                value={form[key]}
                onChange={(e) => setForm((prev) => ({...prev, [key]: e.target.value}))}
                placeholder={opts?.placeholder ?? ""}
                className={`w-full rounded border border-gray-200 bg-white px-2 py-1.5 text-sm focus:border-gray-400 focus:outline-none ${opts?.mono ? "font-mono" : ""}`}
            />
        </div>
    );

    return (
        <div className="rounded-lg border border-gray-200 bg-white">
            <button
                type="button"
                onClick={() => {
                    setOpen((v) => !v);
                    setSaveResult(null);
                }}
                className="flex w-full items-center justify-between px-4 py-3 text-sm font-semibold text-gray-800 hover:bg-gray-50 rounded-lg"
            >
                <span>AWS / S3 Backup Configuration</span>
                <span className="text-gray-400 text-xs">{open ? "▲ collapse" : "▼ expand"}</span>
            </button>

            {open && (
                <div className="border-t border-gray-100 px-4 pb-4 pt-3">
                    <form onSubmit={handleSubmit} className="space-y-3">
                        {configQuery.isLoading && <p className="text-center text-sm text-gray-400">Loading…</p>}
                        {configQuery.data && !configQuery.data.config_file_configured && (
                            <p className="rounded bg-amber-50 p-2.5 text-sm text-amber-700">
                                Node was not started with <span
                                className="font-mono text-xs">--backup-config-file</span>. Config changes will be
                                rejected.
                            </p>
                        )}

                        <div className="grid grid-cols-2 gap-3">
                            {field("Endpoint URL", "aws_endpoint_url", {
                                placeholder: "https://storage.googleapis.com",
                                mono: true
                            })}
                            {field("Region", "aws_default_region", {placeholder: "us-east-1", mono: true})}
                            {field("Access Key ID", "aws_access_key_id", {placeholder: "GOOG1E…", mono: true})}
                            <div>
                                <div className="mb-0.5 flex items-center justify-between">
                                    <label className="text-xs font-medium text-gray-500">Secret Access Key</label>
                                    <button type="button" onClick={() => setShowSecret((v) => !v)}
                                            className="text-xs text-gray-400 hover:text-gray-600">
                                        {showSecret ? "Hide" : "Show"}
                                    </button>
                                </div>
                                <input
                                    type={showSecret ? "text" : "password"}
                                    value={form.aws_secret_access_key}
                                    onChange={(e) => setForm((prev) => ({
                                        ...prev,
                                        aws_secret_access_key: e.target.value
                                    }))}
                                    placeholder="••••••••"
                                    className="w-full rounded border border-gray-200 bg-white px-2 py-1.5 font-mono text-sm focus:border-gray-400 focus:outline-none"
                                />
                            </div>
                            {field("S3 Bucket", "bucket", {placeholder: "my-tigerbeetle-backups", mono: true})}
                            {field("Backup File Path", "backup_file", {
                                placeholder: "./data/0_0.tigerbeetle",
                                mono: true
                            })}
                            {field("Request Checksum Calculation", "aws_request_checksum_calculation", {
                                placeholder: "when_required",
                                mono: true
                            })}
                            {field("Response Checksum Validation", "aws_response_checksum_validation", {
                                placeholder: "when_required",
                                mono: true
                            })}
                        </div>

                        {saveResult && (
                            <p className={`rounded p-2 text-sm ${saveResult.ok ? "bg-green-50 text-green-700" : "bg-red-50 text-red-700"}`}>
                                {saveResult.msg}
                            </p>
                        )}
                        <button
                            type="submit"
                            disabled={modifyMutation.isPending}
                            className="rounded bg-gray-900 px-4 py-1.5 text-sm font-medium text-white hover:bg-gray-800 disabled:opacity-50"
                        >
                            {modifyMutation.isPending ? "Saving…" : "Save Configuration"}
                        </button>
                    </form>
                </div>
            )}
        </div>
    );
}

// ---------------------------------------------------------------------------
// Cron helpers
// ---------------------------------------------------------------------------

function describeCron(pattern: string): string | null {
    const fields = pattern.trim().split(/\s+/);
    if (fields.length !== 6) return null;
    const [sec, min, hour, dom, month, dow] = fields;

    const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

    const num = (s: string) => (/^\d+$/.test(s) ? parseInt(s, 10) : null);
    const stepOf = (s: string) => {
        const m = s.match(/^\*\/(\d+)$/);
        return m ? parseInt(m[1], 10) : null;
    };
    const star = (s: string) => s === "*";
    const pad = (n: number) => String(n).padStart(2, "0");

    const timeStr = (): string | null => {
        const h = num(hour), m = num(min), s = num(sec);
        if (h !== null && m !== null) {
            const base = `${pad(h)}:${pad(m)}`;
            return s !== null && s !== 0 ? `${base}:${pad(s)} UTC` : `${base} UTC`;
        }
        return null;
    };

    const stepSec = stepOf(sec), stepMin = stepOf(min), stepHour = stepOf(hour);
    if (stepSec && star(min) && star(hour) && star(dom) && star(month) && star(dow)) return `Every ${stepSec} second${stepSec > 1 ? "s" : ""}`;
    if (sec === "*" && star(min) && star(hour) && star(dom) && star(month) && star(dow)) return "Every second";
    if (num(sec) === 0 && stepMin && star(hour) && star(dom) && star(month) && star(dow)) return `Every ${stepMin} minute${stepMin > 1 ? "s" : ""}`;
    if (num(sec) === 0 && star(min) && star(hour) && star(dom) && star(month) && star(dow)) return "Every minute";
    if (num(sec) === 0 && num(min) === 0 && stepHour && star(dom) && star(month) && star(dow)) return `Every ${stepHour} hour${stepHour > 1 ? "s" : ""}`;
    if (num(sec) === 0 && num(min) === 0 && star(hour) && star(dom) && star(month) && star(dow)) return "Every hour";

    const dowNum = num(dow);
    if (dowNum !== null && !star(dow) && star(dom) && star(month)) {
        const day = DAYS[dowNum] ?? `day ${dowNum}`;
        const t = timeStr();
        return `Every ${day}${t ? ` at ${t}` : ""}`;
    }
    const monthNum = num(month), domNum = num(dom);
    if (monthNum !== null && domNum !== null) {
        const mo = MONTHS[(monthNum - 1)] ?? `month ${monthNum}`;
        const t = timeStr();
        return `${mo} ${domNum}${t ? ` at ${t}` : ""}`;
    }
    if (domNum !== null && star(month) && star(dow)) {
        const t = timeStr();
        return `Day ${domNum} of every month${t ? ` at ${t}` : ""}`;
    }
    if (star(dom) && star(month) && star(dow)) {
        const t = timeStr();
        return t ? `Every day at ${t}` : "Every day";
    }
    return "Custom schedule";
}

const CRON_FIELDS = [
    {short: "sec", full: "second (0–59)"},
    {short: "min", full: "minute (0–59)"},
    {short: "hour", full: "hour (0–23)"},
    {short: "dom", full: "day of month (1–31)"},
    {short: "mon", full: "month (1–12)"},
    {short: "dow", full: "day of week (0–6, Sun=0)"},
] as const;

function cronFieldAt(value: string, cursorPos: number): number {
    const before = value.slice(0, cursorPos);
    const trimmed = before.replace(/^\s*/, "");
    if (trimmed === "") return 0;
    const parts = trimmed.split(/\s+/);
    return Math.min(/\s$/.test(before) ? parts.length : parts.length - 1, 5);
}

const CRON_PRESETS = [
    ["0 */5 * * * *", "5min"],
    ["0 0 * * * *", "1h"],
    ["0 0 */6 * * *", "6h"],
    ["0 0 0 * * *", "daily"],
    ["0 0 0 * * 0", "weekly"],
] as const;

// ---------------------------------------------------------------------------
// Backup controls
// ---------------------------------------------------------------------------

function NodeBackupControls({nodeId, backupEnabled, currentSchedule, onDone}: {
    nodeId: string;
    backupEnabled: boolean;
    currentSchedule?: string;
    onDone: () => void;
}) {
    const [cron, setCron] = useState(currentSchedule || "0 0 0 * * *");
    const [activeField, setActiveField] = useState<number | null>(null);
    const inputRef = useRef<HTMLInputElement>(null);
    const startBackup = trpc.manager.startBackup.useMutation();
    const stopBackup = trpc.manager.stopBackup.useMutation();
    const triggerBackup = trpc.manager.triggerBackup.useMutation();

    const syncField = (value: string, el: HTMLInputElement) =>
        setActiveField(cronFieldAt(value, el.selectionStart ?? 0));

    if (backupEnabled) {
        return (
            <div className="space-y-3">
                {currentSchedule && (
                    <div className="rounded-lg bg-green-50 px-3 py-2.5 text-sm">
                        <span className="text-green-600 font-medium">Active schedule: </span>
                        <span className="font-mono text-green-800">{currentSchedule}</span>
                        {describeCron(currentSchedule) && (
                            <span className="ml-2 text-green-600">({describeCron(currentSchedule)})</span>
                        )}
                    </div>
                )}
                <div className="flex gap-2">
                    <button
                        onClick={() => stopBackup.mutateAsync({nodeId}).then(onDone)}
                        disabled={stopBackup.isPending}
                        className="rounded border border-red-300 px-4 py-1.5 text-sm font-medium text-red-600 hover:bg-red-50 disabled:opacity-50"
                    >
                        {stopBackup.isPending ? "Stopping…" : "Stop Backup"}
                    </button>
                    <button
                        onClick={() => triggerBackup.mutateAsync({nodeId}).then(onDone)}
                        disabled={triggerBackup.isPending}
                        className="rounded border border-gray-300 px-4 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
                    >
                        {triggerBackup.isPending ? "Triggering…" : "Run Backup Now"}
                    </button>
                </div>
            </div>
        );
    }

    return (
        <div className="space-y-3">
            <div>
                <label className="mb-1.5 block text-sm font-medium text-gray-700">Cron Schedule (UTC)</label>
                <input
                    ref={inputRef}
                    type="text"
                    value={cron}
                    onClick={(e) => syncField(cron, e.currentTarget)}
                    onFocus={(e) => syncField(cron, e.currentTarget)}
                    onBlur={() => setActiveField(null)}
                    onKeyUp={(e) => syncField(cron, e.currentTarget)}
                    onChange={(e) => {
                        setCron(e.target.value);
                        syncField(e.target.value, e.target);
                    }}
                    placeholder="0 0 0 * * *"
                    className="w-full rounded border border-gray-200 px-3 py-2 font-mono text-sm focus:border-gray-400 focus:outline-none"
                />
                <div className="mt-1.5 flex gap-1">
                    {CRON_FIELDS.map((f, i) => (
                        <span key={f.short} title={f.full}
                              className={`flex-1 rounded px-1 py-0.5 text-center font-mono text-xs transition-colors ${activeField === i ? "bg-blue-100 font-semibold text-blue-700" : "bg-gray-100 text-gray-400"}`}>
                            {f.short}
                        </span>
                    ))}
                </div>
                {activeField !== null && <p className="mt-1 text-xs text-blue-600">{CRON_FIELDS[activeField].full}</p>}
                {describeCron(cron) && <p className="mt-0.5 text-xs text-gray-500">{describeCron(cron)}</p>}
                <div className="mt-2 flex flex-wrap gap-1.5">
                    {CRON_PRESETS.map(([pattern, label]) => (
                        <button key={pattern} type="button" onClick={() => setCron(pattern)}
                                className="rounded border border-gray-200 bg-gray-50 px-2 py-0.5 font-mono text-xs hover:bg-gray-100">
                            {label}
                        </button>
                    ))}
                </div>
            </div>
            <div className="flex gap-2">
                <button
                    onClick={() => startBackup.mutateAsync({nodeId, cronSchedule: cron}).then(onDone)}
                    disabled={startBackup.isPending}
                    className="rounded bg-gray-900 px-4 py-1.5 text-sm font-medium text-white hover:bg-gray-800 disabled:opacity-50"
                >
                    {startBackup.isPending ? "Starting…" : "Start Backup"}
                </button>
                <button
                    onClick={() => triggerBackup.mutateAsync({nodeId}).then(onDone)}
                    disabled={triggerBackup.isPending}
                    className="rounded border border-gray-300 px-4 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
                >
                    {triggerBackup.isPending ? "Triggering…" : "Run Backup Now"}
                </button>
            </div>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Tables — full columns
// ---------------------------------------------------------------------------

const PAGE_LIMIT = 50;

const ACCOUNT_HEADERS = [
    "ID", "Ledger", "Code",
    "Debits Pending", "Debits Posted",
    "Credits Pending", "Credits Posted",
    "User Data 128", "User Data 64", "User Data 32",
    "Flags", "Timestamp",
];

const TRANSFER_HEADERS = [
    "ID", "Debit Account", "Credit Account",
    "Amount", "Pending ID",
    "User Data 128", "User Data 64", "User Data 32",
    "Timeout", "Ledger", "Code", "Flags", "Timestamp",
];

interface AccountRecord {
    id: string;
    debits_pending: string;
    debits_posted: string;
    credits_pending: string;
    credits_posted: string;
    user_data_128: string;
    user_data_64: string;
    user_data_32: number;
    ledger: number;
    code: number;
    flags: number;
    timestamp: string;
}

interface TransferRecord {
    id: string;
    debit_account_id: string;
    credit_account_id: string;
    amount: string;
    pending_id: string;
    user_data_128: string;
    user_data_64: string;
    user_data_32: number;
    timeout: number;
    ledger: number;
    code: number;
    flags: number;
    timestamp: string;
}

function AccountRows({accounts, idFormat = "uint128", flagsFmt = "hex"}: {
    accounts: AccountRecord[];
    idFormat?: "uint128" | "uuid";
    flagsFmt?: "hex" | "bin";
}) {
    const fmtId = idFormat === "uuid" ? uint128ToUuid : (id: string) => id;
    const fmtFlags = (f: number) => flagsFmt === "bin"
        ? f.toString(2).padStart(16, "0").replace(/(.{4})/g, "$1 ").trim()
        : `0x${f.toString(16).padStart(4, "0")}`;
    return (
        <>
            {accounts.map((a, i) => (
                <tr key={i} className="hover:bg-gray-50">
                    <td className="px-3 py-1.5 font-mono text-gray-800" title={fmtId(a.id)}>
                        <div className="flex items-center gap-0.5 max-w-[14rem] min-w-0">
                            <span className="truncate min-w-0">{fmtId(a.id)}</span><CopyButton value={fmtId(a.id)}/>
                        </div>
                    </td>
                    <td className="px-3 py-1.5 text-gray-600">{a.ledger}</td>
                    <td className="px-3 py-1.5 text-gray-600">{a.code}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-700 text-right">{a.debits_pending || "0"}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-700 text-right">{a.debits_posted || "0"}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-700 text-right">{a.credits_pending || "0"}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-700 text-right">{a.credits_posted || "0"}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-500 text-xs" title={a.user_data_128}>
                        {a.user_data_128 && a.user_data_128 !== "0"
                            ? <span className="block truncate max-w-[10rem]">{a.user_data_128}</span>
                            : <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 font-mono text-gray-500 text-xs" title={a.user_data_64}>
                        {a.user_data_64 && a.user_data_64 !== "0" ? a.user_data_64 :
                            <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 text-gray-500 text-xs">
                        {a.user_data_32 ? a.user_data_32 : <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 font-mono text-gray-500 text-xs">
                        {a.flags ? fmtFlags(a.flags) :
                            <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 text-gray-500 text-xs whitespace-nowrap">{formatTs(a.timestamp)}</td>
                </tr>
            ))}
        </>
    );
}

function TransferRows({transfers, idFmt = "uint128", debitFmt = "uint128", creditFmt = "uint128", flagsFmt = "hex"}: {
    transfers: TransferRecord[];
    idFmt?: "uint128" | "uuid";
    debitFmt?: "uint128" | "uuid";
    creditFmt?: "uint128" | "uuid";
    flagsFmt?: "hex" | "bin";
}) {
    const fmtId = (id: string, fmt: "uint128" | "uuid") => fmt === "uuid" ? uint128ToUuid(id) : id;
    const fmtFlags = (f: number) => flagsFmt === "bin"
        ? f.toString(2).padStart(16, "0").replace(/(.{4})/g, "$1 ").trim()
        : `0x${f.toString(16).padStart(4, "0")}`;
    return (
        <>
            {transfers.map((t, i) => (
                <tr key={i} className="hover:bg-gray-50">
                    <td className="px-3 py-1.5 font-mono text-gray-800" title={fmtId(t.id, idFmt)}>
                        <div className="flex items-center gap-0.5 max-w-[14rem] min-w-0">
                            <span className="truncate min-w-0">{fmtId(t.id, idFmt)}</span><CopyButton
                            value={fmtId(t.id, idFmt)}/>
                        </div>
                    </td>
                    <td className="px-3 py-1.5 font-mono text-gray-600" title={fmtId(t.debit_account_id, debitFmt)}>
                        <div className="flex items-center gap-0.5 max-w-[12rem] min-w-0">
                            <span className="truncate min-w-0">{fmtId(t.debit_account_id, debitFmt)}</span><CopyButton
                            value={fmtId(t.debit_account_id, debitFmt)}/>
                        </div>
                    </td>
                    <td className="px-3 py-1.5 font-mono text-gray-600" title={fmtId(t.credit_account_id, creditFmt)}>
                        <div className="flex items-center gap-0.5 max-w-[12rem] min-w-0">
                            <span className="truncate min-w-0">{fmtId(t.credit_account_id, creditFmt)}</span><CopyButton
                            value={fmtId(t.credit_account_id, creditFmt)}/>
                        </div>
                    </td>
                    <td className="px-3 py-1.5 font-mono text-gray-700 text-right">{t.amount}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-500 text-xs" title={t.pending_id}>
                        {t.pending_id && t.pending_id !== "0"
                            ? <span className="block truncate max-w-[10rem]">{t.pending_id}</span>
                            : <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 font-mono text-gray-500 text-xs" title={t.user_data_128}>
                        {t.user_data_128 && t.user_data_128 !== "0"
                            ? <span className="block truncate max-w-[10rem]">{t.user_data_128}</span>
                            : <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 font-mono text-gray-500 text-xs">
                        {t.user_data_64 && t.user_data_64 !== "0" ? t.user_data_64 :
                            <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 text-gray-500 text-xs">
                        {t.user_data_32 ? t.user_data_32 : <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 text-gray-500 text-xs">
                        {t.timeout ? t.timeout : <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 text-gray-600">{t.ledger}</td>
                    <td className="px-3 py-1.5 text-gray-600">{t.code}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-500 text-xs">
                        {t.flags ? fmtFlags(t.flags) :
                            <span className="text-gray-300">—</span>}
                    </td>
                    <td className="px-3 py-1.5 text-gray-500 text-xs whitespace-nowrap">{formatTs(t.timestamp)}</td>
                </tr>
            ))}
        </>
    );
}

function RecordTable({headers, children, empty, loading}: {
    headers: React.ReactNode[];
    children: React.ReactNode;
    empty: boolean;
    loading?: boolean;
}) {
    if (loading) return <p className="rounded bg-gray-50 p-4 text-center text-sm text-gray-400">Loading…</p>;
    if (empty) return <p className="rounded bg-gray-50 p-4 text-center text-sm text-gray-400">No records</p>;
    return (
        <div className="overflow-x-auto rounded border border-gray-200">
            <table className="w-full text-xs">
                <thead className="bg-gray-50 sticky top-0">
                <tr>{headers.map((h, i) => (
                    <th key={i} className="px-3 py-2 text-left font-semibold text-gray-600 whitespace-nowrap">{h}</th>
                ))}</tr>
                </thead>
                <tbody className="divide-y divide-gray-100">{children}</tbody>
            </table>
        </div>
    );
}

function Pager({page, hasMore, onPrev, onNext, loading, count}: {
    page: number;
    hasMore: boolean;
    onPrev: () => void;
    onNext: () => void;
    loading: boolean;
    count: number;
}) {
    return (
        <div className="flex items-center justify-between">
            <span className="text-xs text-gray-400">
                {loading ? "Loading…" : `Page ${page + 1} · ${count} records`}
            </span>
            <div className="flex gap-1.5">
                <button disabled={page === 0} onClick={onPrev}
                        className="rounded border border-gray-200 px-3 py-1 text-xs disabled:opacity-40 hover:bg-gray-50">
                    ← Prev
                </button>
                <button disabled={!hasMore} onClick={onNext}
                        className="rounded border border-gray-200 px-3 py-1 text-xs disabled:opacity-40 hover:bg-gray-50">
                    Next →
                </button>
            </div>
        </div>
    );
}

function AccountsTable({nodeId}: { nodeId: string }) {
    const [lsmPage, setLsmPage] = useState(0);
    const [walPage, setWalPage] = useState(0);
    const [idFormat, setIdFormat] = useState<"uint128" | "uuid">("uint128");
    const [flagsFmt, setFlagsFmt] = useState<"hex" | "bin">("hex");

    const lsmQuery = trpc.manager.readLsmAccounts.useQuery({nodeId, page: lsmPage, limit: PAGE_LIMIT});
    const walQuery = trpc.manager.readWalAccounts.useQuery({nodeId, page: walPage, limit: PAGE_LIMIT});

    const lsmAccounts = (lsmQuery.data?.accounts ?? []) as AccountRecord[];
    const walAccounts = (walQuery.data?.accounts ?? []) as AccountRecord[];

    const toggleIdFormat = () => setIdFormat((f) => f === "uint128" ? "uuid" : "uint128");
    const accountHeaders: React.ReactNode[] = [
        <span key="id" className="flex items-center">ID<IdFormatToggle format={idFormat}
                                                                       onToggle={toggleIdFormat}/></span>,
        "Ledger", "Code",
        "Debits Pending", "Debits Posted",
        "Credits Pending", "Credits Posted",
        "User Data 128", "User Data 64", "User Data 32",
        <span key="flags" className="flex items-center">Flags<FlagsFormatToggle format={flagsFmt}
                                                                                onToggle={() => setFlagsFmt((f) => f === "hex" ? "bin" : "hex")}/></span>,
        "Timestamp",
    ];

    return (
        <div className="space-y-8">
            {/* LSM */}
            <div className="space-y-3">
                <div className="flex items-center gap-2">
                    <h3 className="text-base font-semibold text-gray-800">Checkpointed (LSM)</h3>
                    <span className="rounded-full bg-green-100 px-2.5 py-0.5 text-xs font-medium text-green-700">current balances</span>
                    <span className="ml-auto text-sm text-gray-400">
                        {lsmQuery.isLoading ? "…" : `${lsmAccounts.length} records`}
                    </span>
                </div>
                {lsmQuery.isError && (
                    <p className="rounded bg-red-50 p-2.5 text-sm text-red-700">{lsmQuery.error.message}</p>
                )}
                <RecordTable headers={accountHeaders} empty={!lsmQuery.isLoading && lsmAccounts.length === 0}
                             loading={lsmQuery.isLoading}>
                    <AccountRows accounts={lsmAccounts} idFormat={idFormat} flagsFmt={flagsFmt}/>
                </RecordTable>
                <Pager page={lsmPage} hasMore={lsmAccounts.length >= PAGE_LIMIT}
                       onPrev={() => setLsmPage((p) => p - 1)} onNext={() => setLsmPage((p) => p + 1)}
                       loading={lsmQuery.isLoading} count={lsmAccounts.length}/>
            </div>

            {/* WAL */}
            <div className="space-y-3">
                <div className="flex items-center gap-2">
                    <h3 className="text-base font-semibold text-gray-800">Pre-checkpoint (WAL)</h3>
                    <span className="rounded-full bg-amber-100 px-2.5 py-0.5 text-xs font-medium text-amber-700">initial balances</span>
                    <span className="ml-auto text-sm text-gray-400">
                        {walQuery.isLoading ? "…" : `${walAccounts.length} records`}
                    </span>
                </div>
                <p className="text-sm text-gray-400">Created after the last checkpoint (~960 ops). Balances reflect
                    values at creation time.</p>
                {walQuery.isError && (
                    <p className="rounded bg-red-50 p-2.5 text-sm text-red-700">{walQuery.error.message}</p>
                )}
                <RecordTable headers={accountHeaders} empty={!walQuery.isLoading && walAccounts.length === 0}
                             loading={walQuery.isLoading}>
                    <AccountRows accounts={walAccounts} idFormat={idFormat} flagsFmt={flagsFmt}/>
                </RecordTable>
                <Pager page={walPage} hasMore={walAccounts.length >= PAGE_LIMIT}
                       onPrev={() => setWalPage((p) => p - 1)} onNext={() => setWalPage((p) => p + 1)}
                       loading={walQuery.isLoading} count={walAccounts.length}/>
            </div>
        </div>
    );
}

function TransfersTable({nodeId}: { nodeId: string }) {
    const [lsmPage, setLsmPage] = useState(0);
    const [walPage, setWalPage] = useState(0);
    const [idFmt, setIdFmt] = useState<"uint128" | "uuid">("uint128");
    const [debitFmt, setDebitFmt] = useState<"uint128" | "uuid">("uint128");
    const [creditFmt, setCreditFmt] = useState<"uint128" | "uuid">("uint128");
    const [flagsFmt, setFlagsFmt] = useState<"hex" | "bin">("hex");

    const lsmQuery = trpc.manager.readLsmTransfers.useQuery({nodeId, page: lsmPage, limit: PAGE_LIMIT});
    const walQuery = trpc.manager.readWalTransfers.useQuery({nodeId, page: walPage, limit: PAGE_LIMIT});

    const lsmTransfers = (lsmQuery.data?.transfers ?? []) as TransferRecord[];
    const walTransfers = (walQuery.data?.transfers ?? []) as TransferRecord[];

    const transferHeaders: React.ReactNode[] = [
        <span key="id" className="flex items-center">ID<IdFormatToggle format={idFmt}
                                                                       onToggle={() => setIdFmt((f) => f === "uint128" ? "uuid" : "uint128")}/></span>,
        <span key="debit" className="flex items-center">Debit Account<IdFormatToggle format={debitFmt}
                                                                                     onToggle={() => setDebitFmt((f) => f === "uint128" ? "uuid" : "uint128")}/></span>,
        <span key="credit" className="flex items-center">Credit Account<IdFormatToggle format={creditFmt}
                                                                                       onToggle={() => setCreditFmt((f) => f === "uint128" ? "uuid" : "uint128")}/></span>,
        "Amount", "Pending ID",
        "User Data 128", "User Data 64", "User Data 32",
        "Timeout", "Ledger", "Code",
        <span key="flags" className="flex items-center">Flags<FlagsFormatToggle format={flagsFmt}
                                                                                onToggle={() => setFlagsFmt((f) => f === "hex" ? "bin" : "hex")}/></span>,
        "Timestamp",
    ];

    return (
        <div className="space-y-8">
            {/* LSM */}
            <div className="space-y-3">
                <div className="flex items-center gap-2">
                    <h3 className="text-base font-semibold text-gray-800">Checkpointed (LSM)</h3>
                    <span
                        className="rounded-full bg-green-100 px-2.5 py-0.5 text-xs font-medium text-green-700">checkpointed</span>
                    <span className="ml-auto text-sm text-gray-400">
                        {lsmQuery.isLoading ? "…" : `${lsmTransfers.length} records`}
                    </span>
                </div>
                {lsmQuery.isError && (
                    <p className="rounded bg-red-50 p-2.5 text-sm text-red-700">{lsmQuery.error.message}</p>
                )}
                <RecordTable headers={transferHeaders} empty={!lsmQuery.isLoading && lsmTransfers.length === 0}
                             loading={lsmQuery.isLoading}>
                    <TransferRows transfers={lsmTransfers} idFmt={idFmt} debitFmt={debitFmt} creditFmt={creditFmt}
                                  flagsFmt={flagsFmt}/>
                </RecordTable>
                <Pager page={lsmPage} hasMore={lsmTransfers.length >= PAGE_LIMIT}
                       onPrev={() => setLsmPage((p) => p - 1)} onNext={() => setLsmPage((p) => p + 1)}
                       loading={lsmQuery.isLoading} count={lsmTransfers.length}/>
            </div>

            {/* WAL */}
            <div className="space-y-3">
                <div className="flex items-center gap-2">
                    <h3 className="text-base font-semibold text-gray-800">Pre-checkpoint (WAL)</h3>
                    <span className="rounded-full bg-amber-100 px-2.5 py-0.5 text-xs font-medium text-amber-700">pending flush</span>
                    <span className="ml-auto text-sm text-gray-400">
                        {walQuery.isLoading ? "…" : `${walTransfers.length} records`}
                    </span>
                </div>
                <p className="text-sm text-gray-400">Committed after the last checkpoint. Will move to LSM after the
                    next checkpoint.</p>
                {walQuery.isError && (
                    <p className="rounded bg-red-50 p-2.5 text-sm text-red-700">{walQuery.error.message}</p>
                )}
                <RecordTable headers={transferHeaders} empty={!walQuery.isLoading && walTransfers.length === 0}
                             loading={walQuery.isLoading}>
                    <TransferRows transfers={walTransfers} idFmt={idFmt} debitFmt={debitFmt} creditFmt={creditFmt}
                                  flagsFmt={flagsFmt}/>
                </RecordTable>
                <Pager page={walPage} hasMore={walTransfers.length >= PAGE_LIMIT}
                       onPrev={() => setWalPage((p) => p - 1)} onNext={() => setWalPage((p) => p + 1)}
                       loading={walQuery.isLoading} count={walTransfers.length}/>
            </div>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Live log stream
// ---------------------------------------------------------------------------

interface ParsedLogEntry {
    timestamp: string;
    level: string;
    message: string;
    error?: string;
    raw?: string; // fallback if JSON parse fails
}

const LOG_LEVEL_COLOR: Record<string, string> = {
    error: "text-red-400",
    err: "text-red-400",
    warn: "text-yellow-400",
    warning: "text-yellow-400",
    info: "text-blue-400",
    debug: "text-gray-500",
    trace: "text-gray-600",
};

const LOG_LEVEL_ROW_BG: Record<string, string> = {
    error: "bg-red-950/40",
    err: "bg-red-950/40",
    warn: "bg-yellow-950/30",
    warning: "bg-yellow-950/30",
};

function LogStream({nodeId}: { nodeId: string }) {
    const [entries, setEntries] = useState<ParsedLogEntry[]>([]);
    const [connected, setConnected] = useState(false);
    const [paused, setPaused] = useState(false);
    const [levelFilter, setLevelFilter] = useState("all");
    const [tail, setTail] = useState(100);
    const [search, setSearch] = useState("");
    const bottomRef = useRef<HTMLDivElement>(null);
    const pausedRef = useRef(false);
    const autoScrollRef = useRef(true);
    pausedRef.current = paused;

    useEffect(() => {
        setEntries([]);
        setConnected(false);
        const es = new EventSource(`/api/logs/${nodeId}?tail=${tail}`);
        es.onopen = () => setConnected(true);
        es.onmessage = (e: MessageEvent) => {
            if (pausedRef.current) return;
            let entry: ParsedLogEntry;
            try {
                entry = JSON.parse(e.data as string) as ParsedLogEntry;
            } catch {
                entry = {timestamp: "", level: "", message: e.data as string, raw: e.data as string};
            }
            setEntries((prev) => {
                const next = [...prev, entry];
                return next.length > 2000 ? next.slice(-2000) : next;
            });
        };
        es.onerror = () => setConnected(false);
        return () => es.close();
    }, [nodeId, tail]);

    // Auto-scroll when new entries arrive (unless user scrolled up).
    useEffect(() => {
        if (autoScrollRef.current) {
            bottomRef.current?.scrollIntoView({behavior: "smooth"});
        }
    }, [entries]);

    const filtered = entries.filter((e) => {
        const level = (e.level ?? "").toLowerCase();
        if (levelFilter !== "all" && level !== levelFilter) return false;
        if (search && !e.message?.toLowerCase().includes(search.toLowerCase())) return false;
        return true;
    });

    return (
        <div className="space-y-2">
            {/* Toolbar */}
            <div className="flex flex-wrap items-center gap-2">
                <span
                    className={`inline-block h-2 w-2 shrink-0 rounded-full ${connected ? "bg-green-500" : "bg-red-400"}`}/>
                <span className="text-sm text-gray-500 shrink-0">{connected ? "Live" : "Disconnected"}</span>

                <select
                    value={levelFilter}
                    onChange={(e) => setLevelFilter(e.target.value)}
                    className="rounded border border-gray-200 bg-white px-2 py-1 text-xs focus:outline-none"
                >
                    <option value="all">All levels</option>
                    <option value="error">Error</option>
                    <option value="warn">Warn</option>
                    <option value="info">Info</option>
                    <option value="debug">Debug</option>
                    <option value="trace">Trace</option>
                </select>

                <select
                    value={tail}
                    onChange={(e) => setTail(parseInt(e.target.value, 10))}
                    className="rounded border border-gray-200 bg-white px-2 py-1 text-xs focus:outline-none"
                >
                    <option value={50}>Last 50</option>
                    <option value={100}>Last 100</option>
                    <option value={200}>Last 200</option>
                    <option value={500}>Last 500</option>
                </select>

                <input
                    type="text"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    placeholder="Filter messages…"
                    className="rounded border border-gray-200 px-2 py-1 text-xs focus:border-gray-400 focus:outline-none w-40"
                />

                <span className="ml-auto text-xs text-gray-400 shrink-0">
                    {filtered.length}/{entries.length} lines
                    {paused && <span className="ml-1 text-amber-500">paused</span>}
                </span>

                <button
                    onClick={() => setPaused((v) => !v)}
                    className={`rounded border px-3 py-1 text-xs hover:bg-gray-50 ${paused ? "border-amber-300 text-amber-600" : "border-gray-200"}`}
                >
                    {paused ? "Resume" : "Pause"}
                </button>
                <button
                    onClick={() => setEntries([])}
                    className="rounded border border-gray-200 px-3 py-1 text-xs hover:bg-gray-50"
                >
                    Clear
                </button>
            </div>

            {/* Log viewport */}
            <div
                onScroll={(e) => {
                    const el = e.currentTarget;
                    autoScrollRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
                }}
                className="h-[32rem] overflow-y-auto rounded border border-gray-800 bg-gray-950 font-mono text-xs"
            >
                {filtered.length === 0 ? (
                    <p className="p-4 text-gray-600">
                        {connected ? (entries.length === 0 ? "Waiting for log output…" : "No lines match the current filter.") : "Connecting…"}
                    </p>
                ) : (
                    <table className="w-full border-collapse">
                        <tbody>
                        {filtered.map((entry, i) => {
                            const level = (entry.level ?? "").toLowerCase();
                            const levelColor = LOG_LEVEL_COLOR[level] ?? "text-gray-400";
                            const rowBg = LOG_LEVEL_ROW_BG[level] ?? "";
                            const ts = entry.timestamp
                                ? (() => {
                                    try {
                                        return new Date(entry.timestamp).toISOString().slice(11, 23);
                                    } catch {
                                        return entry.timestamp;
                                    }
                                })()
                                : "";
                            return (
                                <tr key={i} className={`leading-5 hover:bg-white/5 ${rowBg}`}>
                                    {ts && (
                                        <td className="px-3 py-0.5 text-gray-600 whitespace-nowrap align-top select-none w-[7rem]">
                                            {ts}
                                        </td>
                                    )}
                                    <td className={`px-1 py-0.5 uppercase align-top whitespace-nowrap select-none w-10 ${levelColor}`}>
                                        {(entry.level ?? "?").slice(0, 5)}
                                    </td>
                                    <td className="px-3 py-0.5 text-gray-200 break-all whitespace-pre-wrap align-top">
                                        {entry.message ?? entry.raw ?? ""}
                                        {entry.error && (
                                            <span className="ml-2 text-red-400">{entry.error}</span>
                                        )}
                                    </td>
                                </tr>
                            );
                        })}
                        </tbody>
                    </table>
                )}
                <div ref={bottomRef}/>
            </div>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Section wrapper
// ---------------------------------------------------------------------------

function Section({title, children, id}: { title: string; children: React.ReactNode; id?: string }) {
    return (
        <section id={id} className="space-y-4">
            <h2 className="text-lg font-semibold text-gray-900 border-b border-gray-200 pb-2">{title}</h2>
            {children}
        </section>
    );
}

function StatCell({label, value, mono, className}: {
    label: string;
    value: React.ReactNode;
    mono?: boolean;
    className?: string
}) {
    return (
        <div className={`rounded-lg border border-gray-200 bg-white px-4 py-3 ${className ?? ""}`}>
            <p className="text-xs font-medium text-gray-400 uppercase tracking-wide">{label}</p>
            <p className={`mt-1 text-sm font-semibold text-gray-800 break-all ${mono ? "font-mono" : ""}`}>{value}</p>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Migration panel
// ---------------------------------------------------------------------------

interface MigrationProgressEvent {
    phase: string;
    imported: string;
    total: string;
    done: boolean;
    error: string;
}

type DetailPanel = "accounts" | "ledgers" | "transfers" | "pending" | null;

interface MigrationAccountFilter {
    id?: string;
    ledger?: number;
    code?: number;
    flags?: number;
    user_data_32?: number;
    user_data_64?: string;
    user_data_128?: string;
}

function MigratePanel({nodeId}: { nodeId: string }) {
    const planQuery = trpc.manager.planMigration.useQuery(
        {nodeId},
        {enabled: false} // manual trigger
    );
    const [newClusterId, setNewClusterId] = useState("");
    const [newAddresses, setNewAddresses] = useState("");
    const [targetClusterId, setTargetClusterId] = useState("");
    const [migrating, setMigrating] = useState(false);
    const [progress, setProgress] = useState<MigrationProgressEvent[]>([]);
    const [migrationDone, setMigrationDone] = useState(false);
    const [migrationError, setMigrationError] = useState<string | null>(null);

    // Drill-down state
    const [activeDetail, setActiveDetail] = useState<DetailPanel>(null);
    const [detailPage, setDetailPage] = useState(0);
    const [accountFilter, setAccountFilter] = useState<MigrationAccountFilter>({});
    const [filterDraft, setFilterDraft] = useState<MigrationAccountFilter>({});

    const DETAIL_LIMIT = 100;

    // Paginated account drill-down query
    const accountsQuery = trpc.manager.getMigrationAccounts.useQuery(
        {nodeId, page: detailPage, limit: DETAIL_LIMIT, filter: accountFilter},
        {enabled: activeDetail === "accounts"}
    );

    // Paginated synthetic transfer drill-down query
    const transfersQuery = trpc.manager.getMigrationSyntheticTransfers.useQuery(
        {nodeId, page: detailPage, limit: DETAIL_LIMIT},
        {enabled: activeDetail === "transfers"}
    );

    // Configured clusters for migration target selection.
    const clustersQuery = trpc.manager.getClusters.useQuery();
    const clusterAddrQuery = trpc.manager.getClusterForMigration.useQuery(
        {clusterId: targetClusterId},
        {enabled: !!targetClusterId}
    );

    // Auto-populate addresses when a cluster is selected.
    useEffect(() => {
        if (clusterAddrQuery.data?.addresses) {
            setNewAddresses(clusterAddrQuery.data.addresses);
        }
    }, [clusterAddrQuery.data]);

    const runPreflight = () => {
        planQuery.refetch();
    };

    const toggleDetail = (panel: DetailPanel) => {
        if (activeDetail === panel) {
            setActiveDetail(null);
        } else {
            setActiveDetail(panel);
            setDetailPage(0);
        }
    };

    const applyFilter = () => {
        setAccountFilter({...filterDraft});
        setDetailPage(0);
    };

    const clearFilter = () => {
        setFilterDraft({});
        setAccountFilter({});
        setDetailPage(0);
    };

    const startMigration = async () => {
        const cid = parseInt(newClusterId, 10);
        if (isNaN(cid) || !newAddresses.trim()) return;

        setMigrating(true);
        setProgress([]);
        setMigrationDone(false);
        setMigrationError(null);

        try {
            const resp = await fetch("/api/migration/execute", {
                method: "POST",
                headers: {"Content-Type": "application/json"},
                body: JSON.stringify({
                    nodeId,
                    newClusterId: cid,
                    newAddresses: newAddresses.trim(),
                }),
            });

            if (!resp.ok) {
                setMigrationError(`HTTP ${resp.status}: ${await resp.text()}`);
                setMigrating(false);
                return;
            }

            const reader = resp.body?.getReader();
            const decoder = new TextDecoder();
            if (!reader) {
                setMigrationError("No response body");
                setMigrating(false);
                return;
            }

            let buffer = "";
            while (true) {
                const {value, done} = await reader.read();
                if (done) break;
                buffer += decoder.decode(value, {stream: true});

                const lines = buffer.split("\n\n");
                buffer = lines.pop() || "";

                for (const line of lines) {
                    const match = line.match(/^data: (.+)$/);
                    if (match) {
                        try {
                            const evt = JSON.parse(match[1]) as MigrationProgressEvent;
                            setProgress((prev) => [...prev, evt]);
                            if (evt.done) setMigrationDone(true);
                            if (evt.error) setMigrationError(evt.error);
                        } catch { /* ignore parse errors */
                        }
                    }
                }
            }
        } catch (e: any) {
            setMigrationError(e.message || "Unknown error");
        } finally {
            setMigrating(false);
        }
    };

    const plan = planQuery.data;
    const latestProgress = progress.length > 0 ? progress[progress.length - 1] : null;

    const cardClass = (panel: DetailPanel) =>
        `rounded-lg border p-3 cursor-pointer transition-colors ${
            activeDetail === panel
                ? "border-blue-400 bg-blue-50"
                : "border-gray-100 bg-gray-50 hover:border-blue-300"
        }`;

    return (
        <div className="space-y-6">
            {/* Step 1: Pre-flight check */}
            <div className="rounded-lg border border-gray-200 bg-white p-6">
                <h3 className="mb-4 text-sm font-semibold text-gray-900">Step 1 — Pre-flight Check</h3>
                <p className="mb-4 text-xs text-gray-500">
                    Reads the data file to count accounts, check pending balances, and estimate synthetic transfers.
                    This is read-only and has no side effects.
                </p>
                <button
                    onClick={runPreflight}
                    disabled={planQuery.isFetching}
                    className="rounded-md bg-gray-900 px-4 py-2 text-sm font-medium text-white hover:bg-gray-800 disabled:opacity-50"
                >
                    {planQuery.isFetching ? "Checking..." : "Run Pre-flight Check"}
                </button>

                {planQuery.isError && (
                    <div className="mt-4 rounded border border-red-200 bg-red-50 p-3 text-xs text-red-700">
                        {planQuery.error.message}
                    </div>
                )}

                {plan && (
                    <div className="mt-4 space-y-3">
                        {/* Clickable stat cards */}
                        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                            <div className={cardClass("accounts")} onClick={() => toggleDetail("accounts")}>
                                <div className="text-xs text-gray-500">Accounts</div>
                                <div className="text-lg font-bold tabular-nums">
                                    {parseInt(plan.accounts, 10).toLocaleString()}
                                </div>
                            </div>
                            <div className={cardClass("ledgers")} onClick={() => toggleDetail("ledgers")}>
                                <div className="text-xs text-gray-500">Ledgers</div>
                                <div className="text-lg font-bold tabular-nums">{plan.ledgers}</div>
                            </div>
                            <div className={cardClass("transfers")} onClick={() => toggleDetail("transfers")}>
                                <div className="text-xs text-gray-500">Synthetic Transfers</div>
                                <div className="text-lg font-bold tabular-nums">
                                    {parseInt(plan.synthetic_transfers, 10).toLocaleString()}
                                </div>
                            </div>
                            <div className={cardClass("pending")} onClick={() => toggleDetail("pending")}>
                                <div className="text-xs text-gray-500">Pending</div>
                                <div
                                    className={`text-lg font-bold tabular-nums ${parseInt(plan.pending_transfers, 10) > 0 ? "text-red-600" : "text-green-600"}`}>
                                    {parseInt(plan.pending_transfers, 10).toLocaleString()}
                                </div>
                            </div>
                        </div>

                        {/* Detail panels */}
                        {activeDetail === "accounts" && (
                            <div className="space-y-3 rounded-lg border border-blue-200 bg-white p-4">
                                <h4 className="text-xs font-semibold text-gray-700">Migration Accounts</h4>
                                {/* Filter bar */}
                                <div className="flex flex-wrap items-end gap-2">
                                    <div>
                                        <label className="block text-[10px] text-gray-500">ID</label>
                                        <input
                                            type="text"
                                            value={filterDraft.id ?? ""}
                                            onChange={(e) => setFilterDraft({
                                                ...filterDraft,
                                                id: e.target.value || undefined
                                            })}
                                            placeholder="exact u128"
                                            className="w-36 rounded border border-gray-200 px-2 py-1 text-xs focus:border-blue-400 focus:outline-none"
                                        />
                                    </div>
                                    <div>
                                        <label className="block text-[10px] text-gray-500">Ledger</label>
                                        <input
                                            type="number"
                                            value={filterDraft.ledger ?? ""}
                                            onChange={(e) => setFilterDraft({
                                                ...filterDraft,
                                                ledger: e.target.value ? parseInt(e.target.value) : undefined
                                            })}
                                            className="w-20 rounded border border-gray-200 px-2 py-1 text-xs focus:border-blue-400 focus:outline-none"
                                        />
                                    </div>
                                    <div>
                                        <label className="block text-[10px] text-gray-500">Code</label>
                                        <input
                                            type="number"
                                            value={filterDraft.code ?? ""}
                                            onChange={(e) => setFilterDraft({
                                                ...filterDraft,
                                                code: e.target.value ? parseInt(e.target.value) : undefined
                                            })}
                                            className="w-20 rounded border border-gray-200 px-2 py-1 text-xs focus:border-blue-400 focus:outline-none"
                                        />
                                    </div>
                                    <div>
                                        <label className="block text-[10px] text-gray-500">Flags</label>
                                        <input
                                            type="number"
                                            value={filterDraft.flags ?? ""}
                                            onChange={(e) => setFilterDraft({
                                                ...filterDraft,
                                                flags: e.target.value ? parseInt(e.target.value) : undefined
                                            })}
                                            className="w-20 rounded border border-gray-200 px-2 py-1 text-xs focus:border-blue-400 focus:outline-none"
                                        />
                                    </div>
                                    <div>
                                        <label className="block text-[10px] text-gray-500">UD32</label>
                                        <input
                                            type="number"
                                            value={filterDraft.user_data_32 ?? ""}
                                            onChange={(e) => setFilterDraft({
                                                ...filterDraft,
                                                user_data_32: e.target.value ? parseInt(e.target.value) : undefined
                                            })}
                                            className="w-20 rounded border border-gray-200 px-2 py-1 text-xs focus:border-blue-400 focus:outline-none"
                                        />
                                    </div>
                                    <div>
                                        <label className="block text-[10px] text-gray-500">UD64</label>
                                        <input
                                            type="text"
                                            value={filterDraft.user_data_64 ?? ""}
                                            onChange={(e) => setFilterDraft({
                                                ...filterDraft,
                                                user_data_64: e.target.value || undefined
                                            })}
                                            className="w-24 rounded border border-gray-200 px-2 py-1 text-xs focus:border-blue-400 focus:outline-none"
                                        />
                                    </div>
                                    <div>
                                        <label className="block text-[10px] text-gray-500">UD128</label>
                                        <input
                                            type="text"
                                            value={filterDraft.user_data_128 ?? ""}
                                            onChange={(e) => setFilterDraft({
                                                ...filterDraft,
                                                user_data_128: e.target.value || undefined
                                            })}
                                            className="w-32 rounded border border-gray-200 px-2 py-1 text-xs focus:border-blue-400 focus:outline-none"
                                        />
                                    </div>
                                    <button onClick={applyFilter}
                                            className="rounded bg-gray-900 px-3 py-1 text-xs font-medium text-white hover:bg-gray-800">
                                        Apply
                                    </button>
                                    <button onClick={clearFilter}
                                            className="rounded border border-gray-300 px-3 py-1 text-xs text-gray-600 hover:bg-gray-50">
                                        Clear
                                    </button>
                                </div>
                                <RecordTable
                                    headers={ACCOUNT_HEADERS}
                                    empty={!accountsQuery.isLoading && (accountsQuery.data?.accounts ?? []).length === 0}
                                    loading={accountsQuery.isLoading}
                                >
                                    <AccountRows accounts={(accountsQuery.data?.accounts ?? []) as AccountRecord[]}/>
                                </RecordTable>
                                <Pager
                                    page={detailPage}
                                    hasMore={(accountsQuery.data?.accounts ?? []).length === DETAIL_LIMIT}
                                    onPrev={() => setDetailPage((p) => Math.max(0, p - 1))}
                                    onNext={() => setDetailPage((p) => p + 1)}
                                    loading={accountsQuery.isLoading}
                                    count={(accountsQuery.data?.accounts ?? []).length}
                                />
                                {accountsQuery.data?.total_count != null && (
                                    <span className="text-[10px] text-gray-400">
                                        Total matching: {parseInt(accountsQuery.data.total_count, 10).toLocaleString()}
                                    </span>
                                )}
                            </div>
                        )}

                        {activeDetail === "ledgers" && (
                            <div className="space-y-3 rounded-lg border border-blue-200 bg-white p-4">
                                <h4 className="text-xs font-semibold text-gray-700">Ledger Summaries</h4>
                                <RecordTable
                                    headers={["Ledger", "Account Count", "Total Debits Posted", "Total Credits Posted"]}
                                    empty={(plan.ledger_summaries ?? []).length === 0}
                                >
                                    {(plan.ledger_summaries ?? []).map((ls, i) => (
                                        <tr key={i} className="hover:bg-gray-50">
                                            <td className="px-3 py-1.5 text-gray-800">{ls.ledger}</td>
                                            <td className="px-3 py-1.5 font-mono text-gray-700 text-right">
                                                {parseInt(ls.account_count, 10).toLocaleString()}
                                            </td>
                                            <td className="px-3 py-1.5 font-mono text-gray-700 text-right">
                                                {ls.total_debits_posted}
                                            </td>
                                            <td className="px-3 py-1.5 font-mono text-gray-700 text-right">
                                                {ls.total_credits_posted}
                                            </td>
                                        </tr>
                                    ))}
                                </RecordTable>
                            </div>
                        )}

                        {activeDetail === "transfers" && (
                            <div className="space-y-3 rounded-lg border border-blue-200 bg-white p-4">
                                <h4 className="text-xs font-semibold text-gray-700">Synthetic Transfers</h4>
                                <RecordTable
                                    headers={["ID", "Debit Account", "Credit Account", "Amount", "Ledger", "Code", "Timestamp"]}
                                    empty={!transfersQuery.isLoading && (transfersQuery.data?.transfers ?? []).length === 0}
                                    loading={transfersQuery.isLoading}
                                >
                                    {(transfersQuery.data?.transfers ?? []).map((t, i) => (
                                        <tr key={i} className="hover:bg-gray-50">
                                            <td className="px-3 py-1.5 font-mono text-gray-800" title={t.id}>
                                                <div className="flex items-center gap-0.5 max-w-[14rem] min-w-0">
                                                    <span className="truncate min-w-0">{t.id}</span><CopyButton
                                                    value={t.id}/>
                                                </div>
                                            </td>
                                            <td className="px-3 py-1.5 font-mono text-gray-600"
                                                title={t.debit_account_id}>
                                                <div className="flex items-center gap-0.5 max-w-[12rem] min-w-0">
                                                    <span
                                                        className="truncate min-w-0">{t.debit_account_id}</span><CopyButton
                                                    value={t.debit_account_id}/>
                                                </div>
                                            </td>
                                            <td className="px-3 py-1.5 font-mono text-gray-600"
                                                title={t.credit_account_id}>
                                                <div className="flex items-center gap-0.5 max-w-[12rem] min-w-0">
                                                    <span
                                                        className="truncate min-w-0">{t.credit_account_id}</span><CopyButton
                                                    value={t.credit_account_id}/>
                                                </div>
                                            </td>
                                            <td className="px-3 py-1.5 font-mono text-gray-700 text-right">{t.amount}</td>
                                            <td className="px-3 py-1.5 text-gray-600">{t.ledger}</td>
                                            <td className="px-3 py-1.5 text-gray-600">{t.code}</td>
                                            <td className="px-3 py-1.5 text-gray-500 text-xs whitespace-nowrap">{formatTs(t.timestamp)}</td>
                                        </tr>
                                    ))}
                                </RecordTable>
                                <Pager
                                    page={detailPage}
                                    hasMore={(transfersQuery.data?.transfers ?? []).length === DETAIL_LIMIT}
                                    onPrev={() => setDetailPage((p) => Math.max(0, p - 1))}
                                    onNext={() => setDetailPage((p) => p + 1)}
                                    loading={transfersQuery.isLoading}
                                    count={(transfersQuery.data?.transfers ?? []).length}
                                />
                                {transfersQuery.data?.total_count != null && (
                                    <span className="text-[10px] text-gray-400">
                                        Total: {parseInt(transfersQuery.data.total_count, 10).toLocaleString()}
                                    </span>
                                )}
                            </div>
                        )}

                        {activeDetail === "pending" && (
                            <div className="space-y-3 rounded-lg border border-blue-200 bg-white p-4">
                                <h4 className="text-xs font-semibold text-gray-700">Accounts with Pending Balances</h4>
                                <RecordTable
                                    headers={ACCOUNT_HEADERS}
                                    empty={(plan.pending_accounts ?? []).length === 0}
                                >
                                    <AccountRows accounts={(plan.pending_accounts ?? []) as AccountRecord[]}/>
                                </RecordTable>
                            </div>
                        )}

                        {plan.safe ? (
                            <div className="rounded border border-green-200 bg-green-50 p-3 text-xs text-green-800">
                                Migration is safe to proceed. No pending transfers found.
                            </div>
                        ) : (
                            <div className="rounded border border-red-200 bg-red-50 p-3 text-xs text-red-700">
                                Migration is NOT safe. {plan.pending_transfers} account(s) have pending balances.
                                Void all pending transfers before proceeding.
                            </div>
                        )}
                    </div>
                )}
            </div>

            {/* Step 2: Execute migration */}
            <div className="rounded-lg border border-gray-200 bg-white p-6">
                <h3 className="mb-4 text-sm font-semibold text-gray-900">Step 2 — Execute Migration</h3>
                <p className="mb-4 text-xs text-gray-500">
                    Reads old data file, connects to the new cluster, and imports all accounts and synthetic transfers.
                    The new cluster must be formatted and running before executing.
                </p>

                <div className="mb-4 space-y-3">
                    {/* Target cluster selector */}
                    <div className="grid gap-3 sm:grid-cols-2">
                        <div>
                            <label className="mb-1 block text-xs font-medium text-gray-700">Target Cluster</label>
                            <select
                                value={targetClusterId}
                                onChange={(e) => {
                                    setTargetClusterId(e.target.value);
                                    setNewAddresses("");
                                }}
                                disabled={migrating}
                                className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm focus:border-gray-900 focus:outline-none focus:ring-1 focus:ring-gray-900 disabled:opacity-50"
                            >
                                <option value="">— select a cluster —</option>
                                {(clustersQuery.data ?? []).map((c) => (
                                    <option key={c.id} value={c.id}>{c.id} ({c.nodeCount} nodes)</option>
                                ))}
                            </select>
                        </div>
                        <div>
                            <label className="mb-1 block text-xs font-medium text-gray-700">
                                TigerBeetle Cluster ID
                                <span className="ml-1 text-gray-400 font-normal">(numeric)</span>
                            </label>
                            <input
                                type="number"
                                value={newClusterId}
                                onChange={(e) => setNewClusterId(e.target.value)}
                                placeholder="0"
                                disabled={migrating}
                                className="w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-gray-900 focus:outline-none focus:ring-1 focus:ring-gray-900 disabled:opacity-50"
                            />
                        </div>
                    </div>

                    {/* Addresses — auto-populated from selected cluster */}
                    <div>
                        <div className="mb-1 flex items-center justify-between">
                            <label className="text-xs font-medium text-gray-700">
                                TigerBeetle Addresses
                                {targetClusterId && clusterAddrQuery.isFetching && (
                                    <span className="ml-2 text-gray-400 font-normal">fetching…</span>
                                )}
                                {targetClusterId && clusterAddrQuery.data && !clusterAddrQuery.isFetching && (
                                    <span className="ml-2 font-normal text-green-600">
                                        {clusterAddrQuery.data.onlineCount}/{clusterAddrQuery.data.nodeCount} nodes online
                                    </span>
                                )}
                            </label>
                            {targetClusterId && (
                                <button
                                    type="button"
                                    onClick={() => clusterAddrQuery.refetch()}
                                    disabled={clusterAddrQuery.isFetching || migrating}
                                    className="text-xs text-gray-400 hover:text-gray-600 disabled:opacity-40"
                                >
                                    Refresh
                                </button>
                            )}
                        </div>
                        <input
                            type="text"
                            value={newAddresses}
                            onChange={(e) => setNewAddresses(e.target.value)}
                            placeholder={targetClusterId ? "Fetching addresses…" : "Select a cluster above, or enter manually: h1:3000,h2:3000"}
                            disabled={migrating}
                            className="w-full rounded-md border border-gray-300 px-3 py-2 font-mono text-sm focus:border-gray-900 focus:outline-none focus:ring-1 focus:ring-gray-900 disabled:opacity-50"
                        />
                        <p className="mt-0.5 text-xs text-gray-400">
                            Auto-filled from target cluster nodes. You can edit manually if needed.
                        </p>
                    </div>
                </div>

                <button
                    onClick={startMigration}
                    disabled={migrating || !newClusterId || !newAddresses.trim() || (plan != null && !plan.safe)}
                    className="rounded-md bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-50"
                >
                    {migrating ? "Migrating..." : "Start Migration"}
                </button>

                {plan != null && !plan.safe && (
                    <p className="mt-2 text-xs text-red-600">
                        Cannot start migration — pending transfers exist. Run pre-flight check after voiding them.
                    </p>
                )}

                {/* Progress display */}
                {(progress.length > 0 || migrating) && (
                    <div className="mt-4 space-y-3">
                        {latestProgress && !latestProgress.done && (
                            <div>
                                <div className="mb-1 flex justify-between text-xs">
                                    <span className="font-medium text-gray-700">
                                        Phase: {latestProgress.phase}
                                    </span>
                                    <span className="tabular-nums text-gray-500">
                                        {parseInt(latestProgress.imported, 10).toLocaleString()} / {parseInt(latestProgress.total, 10).toLocaleString()}
                                    </span>
                                </div>
                                <div className="h-2 w-full overflow-hidden rounded-full bg-gray-200">
                                    <div
                                        className="h-full rounded-full bg-blue-500 transition-all"
                                        style={{
                                            width: `${parseInt(latestProgress.total, 10) > 0
                                                ? (parseInt(latestProgress.imported, 10) / parseInt(latestProgress.total, 10)) * 100
                                                : 0}%`
                                        }}
                                    />
                                </div>
                            </div>
                        )}

                        {migrationDone && (
                            <div className="rounded border border-green-200 bg-green-50 p-3 text-xs text-green-800">
                                Migration completed successfully.
                            </div>
                        )}

                        {migrationError && (
                            <div className="rounded border border-red-200 bg-red-50 p-3 text-xs text-red-700">
                                Migration error: {migrationError}
                            </div>
                        )}

                        {/* Progress log */}
                        <details className="text-xs">
                            <summary className="cursor-pointer text-gray-500 hover:text-gray-700">
                                Progress log ({progress.length} events)
                            </summary>
                            <div
                                className="mt-2 max-h-48 overflow-y-auto rounded border border-gray-200 bg-gray-50 p-2 font-mono">
                                {progress.map((p, i) => (
                                    <div key={i} className="text-gray-600">
                                        [{p.phase}] {p.imported}/{p.total}{p.done ? " DONE" : ""}{p.error ? ` ERROR: ${p.error}` : ""}
                                    </div>
                                ))}
                            </div>
                        </details>
                    </div>
                )}
            </div>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Node detail page
// ---------------------------------------------------------------------------

type PageTab = "overview" | "backup" | "accounts" | "transfers" | "logs" | "migrate";

export default function NodeDetailPage() {
    const params = useParams();
    const router = useRouter();
    const nodeId = typeof params?.nodeId === "string" ? params.nodeId : "";

    const [isAuthenticated, setIsAuthenticated] = useState(false);
    const [tab, setTab] = useState<PageTab>("overview");

    const checkAuth = trpc.manager.checkAuth.useQuery();
    const nodeQuery = trpc.manager.getNodeStatus.useQuery(
        {nodeId},
        {enabled: isAuthenticated && !!nodeId, refetchInterval: 5000}
    );

    useEffect(() => {
        if (checkAuth.data?.isAuthenticated) {
            setIsAuthenticated(true);
        } else if (checkAuth.data && !checkAuth.data.isAuthenticated) {
            router.push("/");
        }
    }, [checkAuth.data, router]);

    if (!isAuthenticated || !nodeId) {
        return (
            <main className="flex min-h-screen items-center justify-center bg-gray-50">
                <p className="text-sm text-gray-500">Checking authentication…</p>
            </main>
        );
    }

    const node = nodeQuery.data;
    const status = node?.status;
    const process = status?.process;
    const backup = status?.backup;
    const capacity = status?.capacity;

    const TABS: { id: PageTab; label: string }[] = [
        {id: "overview", label: "Overview"},
        {id: "backup", label: "Backup"},
        {id: "accounts", label: "Accounts"},
        {id: "transfers", label: "Transfers"},
        {id: "logs", label: "Logs"},
        {id: "migrate", label: "Migrate"},
    ];

    return (
        <main className="min-h-screen bg-gray-50">
            {/* Top bar */}
            <div className="sticky top-0 z-20 border-b border-gray-200 bg-white shadow-sm">
                <div className="mx-auto flex max-w-screen-xl items-center gap-4 px-6 py-3">
                    <button
                        onClick={() => router.push("/")}
                        className="flex items-center gap-1.5 text-sm text-gray-500 hover:text-gray-800"
                    >
                        ← Cluster
                    </button>
                    <span className="text-gray-300">/</span>
                    <div className="flex items-center gap-2.5">
                        {node ? (
                            <span
                                className={`inline-block h-2.5 w-2.5 rounded-full ${node.online ? "bg-green-500" : "bg-red-500"}`}/>
                        ) : null}
                        <h1 className="font-mono text-base font-bold text-gray-900">{nodeId}</h1>
                        {process && <ProcessStateBadge state={process.state}/>}
                    </div>
                    <div className="ml-auto flex items-center gap-2 text-xs text-gray-400">
                        {nodeQuery.isFetching && <span>Refreshing…</span>}
                        <button
                            onClick={() => nodeQuery.refetch()}
                            className="rounded border border-gray-200 px-2 py-1 hover:bg-gray-50"
                        >
                            Refresh
                        </button>
                    </div>
                </div>

                {/* Tabs */}
                <div className="mx-auto flex max-w-screen-xl gap-0 px-6">
                    {TABS.map((t) => (
                        <button
                            key={t.id}
                            onClick={() => setTab(t.id)}
                            className={`border-b-2 px-4 py-2.5 text-sm font-medium transition-colors ${
                                tab === t.id
                                    ? "border-gray-900 text-gray-900"
                                    : "border-transparent text-gray-400 hover:text-gray-600"
                            }`}
                        >
                            {t.label}
                        </button>
                    ))}
                </div>
            </div>

            {/* Content */}
            <div className="mx-auto max-w-screen-xl px-6 py-8 space-y-8">
                {nodeQuery.isLoading && (
                    <div className="rounded-lg border border-gray-200 bg-white p-8 text-center">
                        <p className="text-sm text-gray-500">Loading node status…</p>
                    </div>
                )}
                {nodeQuery.isError && (
                    <div className="rounded-lg border border-red-200 bg-red-50 p-6">
                        <p className="text-sm text-red-700">{nodeQuery.error.message}</p>
                    </div>
                )}
                {node && !node.online && (
                    <div className="rounded-lg border border-red-200 bg-red-50 p-6">
                        <p className="text-sm font-medium text-red-700">Node is offline — cannot
                            reach {node.host}:{node.port}</p>
                    </div>
                )}

                {/* ── Overview ─────────────────────────────────── */}
                {tab === "overview" && (
                    <>
                        {/* Stats grid */}
                        {status && (
                            <Section title="Node Status">
                                <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
                                    <StatCell label="State"
                                              value={process ? <ProcessStateBadge state={process.state}/> : "—"}/>
                                    <StatCell label="PID" value={process?.pid ?? "—"} mono/>
                                    <StatCell label="Uptime" value={formatUptime(status.uptime_seconds)}/>
                                    <StatCell label="Node ID" value={status.node_id} mono/>
                                    <StatCell label="Address" value={process?.address ? `:${process.address}` : "—"}
                                              mono/>
                                    <StatCell label="Backups" value={
                                        <span className={backup?.enabled ? "text-green-600" : "text-gray-400"}>
                                            {backup?.enabled ? "Enabled" : "Disabled"}
                                        </span>
                                    }/>
                                    <StatCell label="Backup Bucket" value={backup?.bucket || "—"} mono/>
                                    <StatCell label="Cron Schedule" value={backup?.cron_schedule || "—"} mono/>
                                </div>
                            </Section>
                        )}

                        {/* Data file capacity */}
                        {capacity && (() => {
                            const total = parseInt(capacity.grid_blocks_total, 10);
                            const used = parseInt(capacity.grid_blocks_used, 10);
                            const fileSize = parseInt(capacity.data_file_size_bytes, 10);
                            const pct = total > 0 ? Math.min(100, (used / total) * 100) : 0;
                            const color =
                                pct >= 90 ? "bg-red-500" : pct >= 70 ? "bg-amber-500" : "bg-blue-500";
                            const colorText =
                                pct >= 90 ? "text-red-700" : pct >= 70 ? "text-amber-700" : "text-blue-700";
                            return (
                                <Section title="Data File Capacity">
                                    <div className="rounded-lg border border-gray-200 bg-white p-4">
                                        <div className="mb-3 flex items-end justify-between">
                                            <div>
                                                <span
                                                    className={`text-2xl font-bold tabular-nums ${colorText}`}>{pct.toFixed(1)}%</span>
                                                <span className="ml-2 text-sm text-gray-500">used</span>
                                            </div>
                                            <span
                                                className="text-sm text-gray-500 font-mono">{formatBytes(fileSize)}</span>
                                        </div>
                                        <div className="h-3 w-full overflow-hidden rounded-full bg-gray-200">
                                            <div
                                                className={`h-full rounded-full transition-all ${color}`}
                                                style={{width: `${pct}%`}}
                                            />
                                        </div>
                                        <div className="mt-2 flex justify-between text-xs text-gray-500">
                                            <span>{used.toLocaleString()} / {total.toLocaleString()} grid blocks</span>
                                            {pct >= 80 && (
                                                <span className="font-medium text-amber-600">
                                                    Consider migration
                                                </span>
                                            )}
                                        </div>
                                    </div>
                                </Section>
                            );
                        })()}

                        {/* Process details */}
                        {process && (
                            <Section title="Process Details">
                                <div className="rounded-lg border border-gray-200 bg-white overflow-hidden">
                                    <table className="w-full text-sm">
                                        <tbody className="divide-y divide-gray-100">
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 w-40 font-medium text-gray-500">Executable</td>
                                            <td className="px-4 py-3 font-mono text-gray-800 break-all">
                                                {process.exe || "—"}
                                                {process.exe && <CopyButton value={process.exe}/>}
                                            </td>
                                        </tr>
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 font-medium text-gray-500">Arguments</td>
                                            <td className="px-4 py-3 font-mono text-gray-700">
                                                {process.args && process.args.length > 0
                                                    ? process.args.map((a, i) => (
                                                        <span key={i}
                                                              className="mr-2 inline-block rounded bg-gray-100 px-1.5 py-0.5 text-xs">{a}</span>
                                                    ))
                                                    : <span className="text-gray-300">—</span>}
                                            </td>
                                        </tr>
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 font-medium text-gray-500">Data File</td>
                                            <td className="px-4 py-3 font-mono text-gray-700 break-all">
                                                {process.data_file || "—"}
                                                {process.data_file && <CopyButton value={process.data_file}/>}
                                            </td>
                                        </tr>
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 font-medium text-gray-500">Listen Address</td>
                                            <td className="px-4 py-3 font-mono text-gray-700">
                                                {process.address ? `:${process.address}` : "—"}
                                            </td>
                                        </tr>
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 font-medium text-gray-500">State</td>
                                            <td className="px-4 py-3">
                                                <ProcessStateBadge state={process.state}/>
                                                <span
                                                    className="ml-2 font-mono text-xs text-gray-400">{process.state}</span>
                                            </td>
                                        </tr>
                                        </tbody>
                                    </table>
                                </div>
                            </Section>
                        )}

                        {/* Backup summary */}
                        {backup && (
                            <Section title="Backup Status">
                                <div className="rounded-lg border border-gray-200 bg-white overflow-hidden">
                                    <table className="w-full text-sm">
                                        <tbody className="divide-y divide-gray-100">
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 w-48 font-medium text-gray-500">Status</td>
                                            <td className="px-4 py-3">
                                                {backup.enabled
                                                    ? <span className="font-medium text-green-600">Enabled</span>
                                                    : <span className="text-gray-400">Disabled</span>}
                                            </td>
                                        </tr>
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 font-medium text-gray-500">Cron Schedule</td>
                                            <td className="px-4 py-3 font-mono text-gray-700">
                                                {backup.cron_schedule || <span className="text-gray-300">—</span>}
                                                {backup.cron_schedule && describeCron(backup.cron_schedule) && (
                                                    <span
                                                        className="ml-2 text-xs text-gray-400">({describeCron(backup.cron_schedule)})</span>
                                                )}
                                            </td>
                                        </tr>
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 font-medium text-gray-500">S3 Bucket</td>
                                            <td className="px-4 py-3 font-mono text-gray-700">{backup.bucket ||
                                                <span className="text-gray-300">—</span>}</td>
                                        </tr>
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 font-medium text-gray-500">Last Backup</td>
                                            <td className="px-4 py-3 text-gray-700">
                                                {backup.last_backup_at
                                                    ? new Date(backup.last_backup_at).toLocaleString()
                                                    : <span className="text-gray-300">Never</span>}
                                            </td>
                                        </tr>
                                        <tr className="hover:bg-gray-50">
                                            <td className="px-4 py-3 font-medium text-gray-500">Last Error</td>
                                            <td className="px-4 py-3 text-red-600 text-sm font-mono">
                                                {backup.last_error || <span className="text-gray-300">—</span>}
                                            </td>
                                        </tr>
                                        </tbody>
                                    </table>
                                </div>
                            </Section>
                        )}
                    </>
                )}

                {/* ── Backup ───────────────────────────────────── */}
                {tab === "backup" && (
                    <>
                        {node?.online && status ? (
                            <>
                                <Section title="Backup Controls">
                                    {backup?.last_backup_at && (
                                        <div className="rounded-lg bg-gray-50 px-4 py-3 text-sm">
                                            <span className="text-gray-500">Last backup: </span>
                                            <span
                                                className="font-mono">{new Date(backup.last_backup_at).toLocaleString()}</span>
                                        </div>
                                    )}
                                    {backup?.last_error && (
                                        <div className="rounded-lg bg-red-50 px-4 py-3 text-sm text-red-700 font-mono">
                                            {backup.last_error}
                                        </div>
                                    )}
                                    <NodeBackupControls
                                        nodeId={nodeId}
                                        backupEnabled={backup?.enabled ?? false}
                                        currentSchedule={backup?.cron_schedule}
                                        onDone={() => nodeQuery.refetch()}
                                    />
                                </Section>

                                <Section title="AWS / S3 Configuration">
                                    <BackupConfigEditor nodeId={nodeId}/>
                                </Section>

                                <div className="rounded-lg border border-amber-200 bg-amber-50 p-3">
                                    <p className="text-sm text-amber-900">
                                        <strong>Timezone:</strong> All cron schedules run in UTC. Convert your local
                                        time accordingly.
                                    </p>
                                </div>
                            </>
                        ) : (
                            <div className="rounded-lg border border-red-200 bg-red-50 p-6">
                                <p className="text-sm text-red-700">Node is offline — backup controls unavailable.</p>
                            </div>
                        )}
                    </>
                )}

                {/* ── Accounts ─────────────────────────────────── */}
                {tab === "accounts" && (
                    <Section title="Accounts">
                        <AccountsTable nodeId={nodeId}/>
                    </Section>
                )}

                {/* ── Transfers ────────────────────────────────── */}
                {tab === "transfers" && (
                    <Section title="Transfers">
                        <TransfersTable nodeId={nodeId}/>
                    </Section>
                )}

                {/* ── Logs ─────────────────────────────────────── */}
                {tab === "logs" && (
                    <Section title="Live Logs">
                        <LogStream nodeId={nodeId}/>
                    </Section>
                )}

                {/* ── Migrate ──────────────────────────────────── */}
                {tab === "migrate" && (
                    <Section title="Cluster Migration">
                        <MigratePanel nodeId={nodeId}/>
                    </Section>
                )}
            </div>
        </main>
    );
}
