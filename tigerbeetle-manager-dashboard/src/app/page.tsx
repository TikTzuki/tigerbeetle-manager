"use client";

import {trpc} from "@/trpc/client";
import {useEffect, useRef, useState} from "react";

function formatUptime(seconds: string | number): string {
    const s = typeof seconds === "string" ? parseInt(seconds, 10) : seconds;
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    return `${h}h ${m}m`;
}

function truncId(id: string, len = 16): string {
    return id.length > len ? id.slice(0, len) + "…" : id;
}

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
            className={`inline-block rounded-full px-2 py-0.5 text-xs font-medium ${colors[state] || "bg-gray-100 text-gray-600"}`}>
            {labels[state] || "Unknown"}
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
        } catch (err: any) {
            setSaveResult({ok: false, msg: err.message ?? "Unknown error"});
        }
    };

    const field = (label: string, key: keyof BackupConfigForm, opts?: {
        placeholder?: string;
        isSecret?: boolean;
        mono?: boolean
    }) => (
        <div>
            <label className="mb-0.5 block text-xs text-gray-500">{label}</label>
            <input
                type={opts?.isSecret && !showSecret ? "password" : "text"}
                value={form[key]}
                onChange={(e) => setForm((prev) => ({...prev, [key]: e.target.value}))}
                placeholder={opts?.placeholder ?? ""}
                className={`w-full rounded border border-gray-200 bg-white px-2 py-1 text-xs focus:border-gray-400 focus:outline-none ${opts?.mono ? "font-mono" : ""}`}
            />
        </div>
    );

    return (
        <div className="border-t border-gray-100 pt-2">
            <button
                type="button"
                onClick={() => {
                    setOpen((v) => !v);
                    setSaveResult(null);
                }}
                className="flex w-full items-center justify-between rounded px-2 py-1 text-xs font-medium text-gray-700 hover:bg-gray-50"
            >
                <span>AWS / S3 Backup Config</span>
                <span className="text-gray-400">{open ? "▲" : "▼"}</span>
            </button>

            {open && (
                <form onSubmit={handleSubmit} className="mt-2 space-y-2">
                    {configQuery.isLoading && <p className="text-center text-xs text-gray-400">Loading...</p>}
                    {configQuery.data && !configQuery.data.config_file_configured && (
                        <p className="rounded bg-amber-50 p-2 text-xs text-amber-700">
                            Node was not started with <span className="font-mono">--backup-config-file</span>. Config
                            changes will be rejected.
                        </p>
                    )}
                    {field("Endpoint URL", "aws_endpoint_url", {
                        placeholder: "https://storage.googleapis.com",
                        mono: true
                    })}
                    {field("Access Key ID", "aws_access_key_id", {placeholder: "GOOG1E…", mono: true})}
                    <div>
                        <div className="mb-0.5 flex items-center justify-between">
                            <label className="text-xs text-gray-500">Secret Access Key</label>
                            <button type="button" onClick={() => setShowSecret((v) => !v)}
                                    className="text-xs text-gray-400 hover:text-gray-600">
                                {showSecret ? "Hide" : "Show"}
                            </button>
                        </div>
                        <input
                            type={showSecret ? "text" : "password"}
                            value={form.aws_secret_access_key}
                            onChange={(e) => setForm((prev) => ({...prev, aws_secret_access_key: e.target.value}))}
                            placeholder="••••••••"
                            className="w-full rounded border border-gray-200 bg-white px-2 py-1 font-mono text-xs focus:border-gray-400 focus:outline-none"
                        />
                    </div>
                    {field("Region", "aws_default_region", {placeholder: "us-east-1", mono: true})}
                    {field("S3 Bucket", "bucket", {placeholder: "my-tigerbeetle-backups", mono: true})}
                    {field("Backup File Path", "backup_file", {placeholder: "./data/0_0.tigerbeetle", mono: true})}
                    {field("Request Checksum Calculation", "aws_request_checksum_calculation", {
                        placeholder: "when_required",
                        mono: true
                    })}
                    {field("Response Checksum Validation", "aws_response_checksum_validation", {
                        placeholder: "when_required",
                        mono: true
                    })}

                    {saveResult && (
                        <p className={`rounded p-1.5 text-xs ${saveResult.ok ? "bg-green-50 text-green-700" : "bg-red-50 text-red-700"}`}>
                            {saveResult.msg}
                        </p>
                    )}
                    <button
                        type="submit"
                        disabled={modifyMutation.isPending}
                        className="w-full rounded bg-gray-900 px-3 py-1 text-xs font-medium text-white hover:bg-gray-800 disabled:opacity-50"
                    >
                        {modifyMutation.isPending ? "Saving..." : "Save Config"}
                    </button>
                </form>
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
// Per-node backup controls
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

    const syncField = (value: string, el: HTMLInputElement) => setActiveField(cronFieldAt(value, el.selectionStart ?? 0));

    if (backupEnabled) {
        return (
            <div className="space-y-2">
                {currentSchedule && (
                    <div className="flex items-center justify-between rounded bg-green-50 px-2 py-1.5 text-xs">
                        <span className="text-green-700">Schedule</span>
                        <span className="font-mono text-green-800">{currentSchedule}</span>
                    </div>
                )}
                <button
                    onClick={() => stopBackup.mutateAsync({nodeId}).then(onDone)}
                    disabled={stopBackup.isPending}
                    className="w-full rounded border border-red-300 px-3 py-1 text-xs font-medium text-red-600 hover:bg-red-50 disabled:opacity-50"
                >
                    {stopBackup.isPending ? "Stopping..." : "Stop Backup"}
                </button>
                <button
                    onClick={() => triggerBackup.mutateAsync({nodeId}).then(onDone)}
                    disabled={triggerBackup.isPending}
                    className="w-full rounded border border-gray-300 px-3 py-1 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
                >
                    {triggerBackup.isPending ? "Triggering..." : "Trigger One-off Backup"}
                </button>
            </div>
        );
    }

    return (
        <div className="space-y-2">
            <div>
                <label className="mb-1 block text-xs text-gray-500">Cron Schedule (UTC)</label>
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
                    className="w-full rounded border border-gray-200 px-2 py-1 font-mono text-xs focus:border-gray-400 focus:outline-none"
                />
                <div className="mt-1 flex gap-1">
                    {CRON_FIELDS.map((f, i) => (
                        <span key={f.short} title={f.full}
                              className={`flex-1 rounded px-1 py-0.5 text-center font-mono text-xs transition-colors ${activeField === i ? "bg-blue-100 font-semibold text-blue-700" : "bg-gray-100 text-gray-400"}`}>
                            {f.short}
                        </span>
                    ))}
                </div>
                {activeField !== null &&
                    <p className="mt-0.5 text-xs text-blue-600">{CRON_FIELDS[activeField].full}</p>}
                {describeCron(cron) && <p className="mt-0.5 text-xs text-gray-500">{describeCron(cron)}</p>}
                <div className="mt-1 flex flex-wrap gap-1">
                    {CRON_PRESETS.map(([pattern, label]) => (
                        <button key={pattern} type="button" onClick={() => setCron(pattern)}
                                className="rounded border border-gray-200 bg-gray-50 px-1.5 py-0.5 font-mono text-xs hover:bg-gray-100">
                            {label}
                        </button>
                    ))}
                </div>
            </div>
            <button
                onClick={() => startBackup.mutateAsync({nodeId, cronSchedule: cron}).then(onDone)}
                disabled={startBackup.isPending}
                className="w-full rounded bg-gray-900 px-3 py-1 text-xs font-medium text-white hover:bg-gray-800 disabled:opacity-50"
            >
                {startBackup.isPending ? "Starting..." : "Start Backup"}
            </button>
            <button
                onClick={() => triggerBackup.mutateAsync({nodeId}).then(onDone)}
                disabled={triggerBackup.isPending}
                className="w-full rounded border border-gray-300 px-3 py-1 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
            >
                {triggerBackup.isPending ? "Triggering..." : "Trigger One-off Backup"}
            </button>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Shared table primitives
// ---------------------------------------------------------------------------

const ACCOUNT_HEADERS = ["ID", "Ledger", "Code", "Debits Posted", "Credits Posted", "Timestamp"];
const TRANSFER_HEADERS = ["ID", "From", "To", "Amount", "Ledger", "Timestamp"];
const PAGE_LIMIT = 50;

function AccountRows({accounts}: {
    accounts: Array<{
        id: string;
        ledger: number;
        code: number;
        debits_posted: string;
        credits_posted: string;
        timestamp: string
    }>
}) {
    return (
        <>
            {accounts.map((a, i) => (
                <tr key={i} className="hover:bg-gray-50">
                    <td className="px-3 py-1.5 font-mono text-gray-800" title={a.id}>{truncId(a.id)}</td>
                    <td className="px-3 py-1.5 text-gray-600">{a.ledger}</td>
                    <td className="px-3 py-1.5 text-gray-600">{a.code}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-700">{a.debits_posted}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-700">{a.credits_posted}</td>
                    <td className="px-3 py-1.5 text-gray-500">
                        {Number(a.timestamp) > 0
                            ? new Date(Number(a.timestamp) / 1_000_000).toISOString()
                            : <span className="text-gray-300">—</span>}
                    </td>
                </tr>
            ))}
        </>
    );
}

function TransferRows({transfers}: {
    transfers: Array<{
        id: string;
        debit_account_id: string;
        credit_account_id: string;
        amount: string;
        ledger: number;
        timestamp: string
    }>
}) {
    return (
        <>
            {transfers.map((t, i) => (
                <tr key={i} className="hover:bg-gray-50">
                    <td className="px-3 py-1.5 font-mono text-gray-800" title={t.id}>{truncId(t.id)}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-600"
                        title={t.debit_account_id}>{truncId(t.debit_account_id)}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-600"
                        title={t.credit_account_id}>{truncId(t.credit_account_id)}</td>
                    <td className="px-3 py-1.5 font-mono text-gray-700">{t.amount}</td>
                    <td className="px-3 py-1.5 text-gray-600">{t.ledger}</td>
                    <td className="px-3 py-1.5 text-gray-500">
                        {Number(t.timestamp) > 0
                            ? new Date(Number(t.timestamp) / 1_000_000).toISOString()
                            : <span className="text-gray-300">—</span>}
                    </td>
                </tr>
            ))}
        </>
    );
}

function RecordTable({headers, children, empty}: { headers: string[]; children: React.ReactNode; empty: boolean }) {
    if (empty) return <p className="rounded bg-gray-50 p-4 text-center text-xs text-gray-400">No records</p>;
    return (
        <div className="overflow-x-auto rounded border border-gray-200">
            <table className="w-full text-xs">
                <thead className="bg-gray-50">
                <tr>{headers.map((h) => <th key={h}
                                            className="px-3 py-2 text-left font-medium text-gray-600">{h}</th>)}</tr>
                </thead>
                <tbody className="divide-y divide-gray-100">{children}</tbody>
            </table>
        </div>
    );
}

function Pager({page, hasMore, onPrev, onNext, loading}: {
    page: number;
    hasMore: boolean;
    onPrev: () => void;
    onNext: () => void;
    loading: boolean
}) {
    return (
        <div className="flex items-center justify-between">
            <span className="text-xs text-gray-400">{loading ? "Loading…" : `Page ${page + 1}`}</span>
            <div className="flex gap-1">
                <button disabled={page === 0} onClick={onPrev}
                        className="rounded border border-gray-200 px-2 py-0.5 text-xs disabled:opacity-40 hover:bg-gray-50">←
                    Prev
                </button>
                <button disabled={!hasMore} onClick={onNext}
                        className="rounded border border-gray-200 px-2 py-0.5 text-xs disabled:opacity-40 hover:bg-gray-50">Next
                    →
                </button>
            </div>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Accounts table (LSM + WAL sections)
// ---------------------------------------------------------------------------

function AccountsTable({nodeId}: { nodeId: string }) {
    const [lsmPage, setLsmPage] = useState(0);
    const [walPage, setWalPage] = useState(0);

    const lsmQuery = trpc.manager.readLsmAccounts.useQuery({nodeId, page: lsmPage, limit: PAGE_LIMIT});
    const walQuery = trpc.manager.readWalAccounts.useQuery({nodeId, page: walPage, limit: PAGE_LIMIT});

    const lsmAccounts = lsmQuery.data?.accounts ?? [];
    const walAccounts = walQuery.data?.accounts ?? [];

    return (
        <div className="space-y-6">
            <div className="space-y-2">
                <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-gray-800">Checkpointed (LSM)</h3>
                    <span
                        className="rounded-full bg-green-100 px-2 py-0.5 text-xs text-green-700">current balances</span>
                    <span
                        className="ml-auto text-xs text-gray-400">{lsmQuery.isLoading ? "…" : `${lsmAccounts.length} records`}</span>
                </div>
                {lsmQuery.isError &&
                    <p className="rounded bg-red-50 p-2 text-xs text-red-700">{lsmQuery.error.message}</p>}
                <RecordTable headers={ACCOUNT_HEADERS} empty={!lsmQuery.isLoading && lsmAccounts.length === 0}>
                    <AccountRows accounts={lsmAccounts}/>
                </RecordTable>
                <Pager page={lsmPage} hasMore={lsmAccounts.length >= PAGE_LIMIT} onPrev={() => setLsmPage((p) => p - 1)}
                       onNext={() => setLsmPage((p) => p + 1)} loading={lsmQuery.isLoading}/>
            </div>

            <div className="space-y-2">
                <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-gray-800">Pre-checkpoint (WAL)</h3>
                    <span
                        className="rounded-full bg-amber-100 px-2 py-0.5 text-xs text-amber-700">initial balances</span>
                    <span
                        className="ml-auto text-xs text-gray-400">{walQuery.isLoading ? "…" : `${walAccounts.length} records`}</span>
                </div>
                <p className="text-xs text-gray-400">Created after the last checkpoint (~960 ops). Balances reflect
                    values at creation time.</p>
                {walQuery.isError &&
                    <p className="rounded bg-red-50 p-2 text-xs text-red-700">{walQuery.error.message}</p>}
                <RecordTable headers={ACCOUNT_HEADERS} empty={!walQuery.isLoading && walAccounts.length === 0}>
                    <AccountRows accounts={walAccounts}/>
                </RecordTable>
                <Pager page={walPage} hasMore={walAccounts.length >= PAGE_LIMIT} onPrev={() => setWalPage((p) => p - 1)}
                       onNext={() => setWalPage((p) => p + 1)} loading={walQuery.isLoading}/>
            </div>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Transfers table (LSM + WAL sections)
// ---------------------------------------------------------------------------

function TransfersTable({nodeId}: { nodeId: string }) {
    const [lsmPage, setLsmPage] = useState(0);
    const [walPage, setWalPage] = useState(0);

    const lsmQuery = trpc.manager.readLsmTransfers.useQuery({nodeId, page: lsmPage, limit: PAGE_LIMIT});
    const walQuery = trpc.manager.readWalTransfers.useQuery({nodeId, page: walPage, limit: PAGE_LIMIT});

    const lsmTransfers = lsmQuery.data?.transfers ?? [];
    const walTransfers = walQuery.data?.transfers ?? [];

    return (
        <div className="space-y-6">
            <div className="space-y-2">
                <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-gray-800">Checkpointed (LSM)</h3>
                    <span className="rounded-full bg-green-100 px-2 py-0.5 text-xs text-green-700">checkpointed</span>
                    <span
                        className="ml-auto text-xs text-gray-400">{lsmQuery.isLoading ? "…" : `${lsmTransfers.length} records`}</span>
                </div>
                {lsmQuery.isError &&
                    <p className="rounded bg-red-50 p-2 text-xs text-red-700">{lsmQuery.error.message}</p>}
                <RecordTable headers={TRANSFER_HEADERS} empty={!lsmQuery.isLoading && lsmTransfers.length === 0}>
                    <TransferRows transfers={lsmTransfers}/>
                </RecordTable>
                <Pager page={lsmPage} hasMore={lsmTransfers.length >= PAGE_LIMIT}
                       onPrev={() => setLsmPage((p) => p - 1)} onNext={() => setLsmPage((p) => p + 1)}
                       loading={lsmQuery.isLoading}/>
            </div>

            <div className="space-y-2">
                <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-gray-800">Pre-checkpoint (WAL)</h3>
                    <span className="rounded-full bg-amber-100 px-2 py-0.5 text-xs text-amber-700">pending flush</span>
                    <span
                        className="ml-auto text-xs text-gray-400">{walQuery.isLoading ? "…" : `${walTransfers.length} records`}</span>
                </div>
                <p className="text-xs text-gray-400">Committed after the last checkpoint. Will move to LSM after the
                    next checkpoint.</p>
                {walQuery.isError &&
                    <p className="rounded bg-red-50 p-2 text-xs text-red-700">{walQuery.error.message}</p>}
                <RecordTable headers={TRANSFER_HEADERS} empty={!walQuery.isLoading && walTransfers.length === 0}>
                    <TransferRows transfers={walTransfers}/>
                </RecordTable>
                <Pager page={walPage} hasMore={walTransfers.length >= PAGE_LIMIT}
                       onPrev={() => setWalPage((p) => p - 1)} onNext={() => setWalPage((p) => p + 1)}
                       loading={walQuery.isLoading}/>
            </div>
        </div>
    );
}

// ---------------------------------------------------------------------------
// Node detail drawer
// ---------------------------------------------------------------------------

type DrawerTab = "backup" | "accounts" | "transfers";

function NodeDrawer({node, onClose, onRefresh}: {
    node: any;
    onClose: () => void;
    onRefresh: () => void;
}) {
    const [tab, setTab] = useState<DrawerTab>("backup");

    return (
        <>
            {/* Backdrop */}
            <div className="fixed inset-0 z-40 bg-black/20" onClick={onClose}/>

            {/* Panel */}
            <div
                className="fixed right-0 top-0 z-50 flex h-full w-full max-w-2xl flex-col border-l border-gray-200 bg-white shadow-xl">
                {/* Header */}
                <div className="flex items-center justify-between border-b border-gray-200 px-5 py-4">
                    <div className="flex items-center gap-3">
                        <span
                            className={`inline-block h-2.5 w-2.5 rounded-full ${node.online ? "bg-green-500" : "bg-red-500"}`}/>
                        <h2 className="font-mono text-base font-semibold">{node.id}</h2>
                        {node.status?.process && <ProcessStateBadge state={node.status.process.state}/>}
                    </div>
                    <button onClick={onClose}
                            className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600">
                        <svg xmlns="http://www.w3.org/2000/svg" className="h-5 w-5" viewBox="0 0 20 20"
                             fill="currentColor">
                            <path fillRule="evenodd"
                                  d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z"
                                  clipRule="evenodd"/>
                        </svg>
                    </button>
                </div>

                {/* Quick stats */}
                {node.online && node.status && (
                    <div
                        className="grid grid-cols-4 divide-x divide-gray-100 border-b border-gray-100 text-center text-xs">
                        <div className="px-3 py-2">
                            <p className="text-gray-400">PID</p>
                            <p className="font-mono font-medium">{node.status.process?.pid || "—"}</p>
                        </div>
                        <div className="px-3 py-2">
                            <p className="text-gray-400">Port</p>
                            <p className="font-mono font-medium">:{node.status.process?.address || "—"}</p>
                        </div>
                        <div className="px-3 py-2">
                            <p className="text-gray-400">Uptime</p>
                            <p className="font-medium">{formatUptime(node.status.uptime_seconds)}</p>
                        </div>
                        <div className="px-3 py-2">
                            <p className="text-gray-400">Backups</p>
                            <p className={`font-medium ${node.status.backup?.enabled ? "text-green-600" : "text-gray-400"}`}>
                                {node.status.backup?.enabled ? "On" : "Off"}
                            </p>
                        </div>
                    </div>
                )}

                {/* Tabs */}
                <div className="flex border-b border-gray-200 px-5">
                    {(["backup", "accounts", "transfers"] as DrawerTab[]).map((t) => (
                        <button key={t} onClick={() => setTab(t)}
                                className={`mr-4 border-b-2 py-2.5 text-sm font-medium capitalize transition-colors ${tab === t ? "border-gray-900 text-gray-900" : "border-transparent text-gray-400 hover:text-gray-600"}`}>
                            {t}
                        </button>
                    ))}
                </div>

                {/* Tab content */}
                <div className="flex-1 overflow-y-auto p-5">
                    {tab === "backup" && (
                        <div className="space-y-4">
                            {!node.online && (
                                <p className="rounded bg-red-50 p-3 text-sm text-red-600">
                                    Node is offline — cannot reach {node.host}:{node.port}
                                </p>
                            )}
                            {node.online && node.status && (
                                <>
                                    {node.status.backup?.last_backup_at && (
                                        <div className="rounded bg-gray-50 p-3 text-xs">
                                            <span className="text-gray-500">Last backup: </span>
                                            <span
                                                className="font-mono">{new Date(node.status.backup.last_backup_at).toLocaleString()}</span>
                                        </div>
                                    )}
                                    {node.status.backup?.last_error && (
                                        <div
                                            className="rounded bg-red-50 p-3 text-xs text-red-700">{node.status.backup.last_error}</div>
                                    )}
                                    <NodeBackupControls
                                        nodeId={node.id}
                                        backupEnabled={node.status.backup?.enabled ?? false}
                                        currentSchedule={node.status.backup?.cron_schedule}
                                        onDone={onRefresh}
                                    />
                                    <BackupConfigEditor nodeId={node.id}/>
                                </>
                            )}
                        </div>
                    )}

                    {tab === "accounts" && <AccountsTable nodeId={node.id}/>}
                    {tab === "transfers" && <TransfersTable nodeId={node.id}/>}
                </div>
            </div>
        </>
    );
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

export default function Home() {
    const [secretKey, setSecretKey] = useState("");
    const [isAuthenticated, setIsAuthenticated] = useState(false);
    const [selectedNode, setSelectedNode] = useState<string | null>(null);

    const checkAuth = trpc.manager.checkAuth.useQuery();
    const login = trpc.manager.login.useMutation();
    const logout = trpc.manager.logout.useMutation();
    const cluster = trpc.manager.getClusterStatus.useQuery(undefined, {
        enabled: isAuthenticated,
        refetchInterval: 5000,
    });

    useEffect(() => {
        if (checkAuth.data?.isAuthenticated) setIsAuthenticated(true);
    }, [checkAuth.data]);

    const handleLogin = async (e: React.FormEvent) => {
        e.preventDefault();
        const result = await login.mutateAsync({secretKey});
        if (result.success) {
            setIsAuthenticated(true);
            setSecretKey("");
            checkAuth.refetch();
        }
    };

    const handleLogout = async () => {
        await logout.mutateAsync();
        setIsAuthenticated(false);
        checkAuth.refetch();
    };

    if (!isAuthenticated) {
        return (
            <main className="flex min-h-screen items-center justify-center bg-gray-50">
                <div className="w-full max-w-sm space-y-6 rounded-lg border border-gray-200 bg-white p-8 shadow-sm">
                    <div className="space-y-2 text-center">
                        <h1 className="text-2xl font-semibold tracking-tight">TigerBeetle Manager</h1>
                        <p className="text-sm text-gray-500">Cluster Dashboard — Enter admin secret key</p>
                    </div>
                    <form onSubmit={handleLogin} className="space-y-4">
                        <input
                            type="password"
                            value={secretKey}
                            onChange={(e) => setSecretKey(e.target.value)}
                            placeholder="Admin Secret Key"
                            className="w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-gray-900 focus:outline-none focus:ring-1 focus:ring-gray-900"
                            required
                        />
                        <button
                            type="submit"
                            disabled={login.isPending}
                            className="w-full rounded-md bg-gray-900 px-4 py-2 text-sm font-medium text-white hover:bg-gray-800 disabled:opacity-50"
                        >
                            {login.isPending ? "Signing in..." : "Sign in"}
                        </button>
                    </form>
                    {login.isError && <p className="text-center text-sm text-red-600">Invalid secret key</p>}
                </div>
            </main>
        );
    }

    const nodes = cluster.data || [];
    const onlineCount = nodes.filter((n) => n.online).length;
    const selectedNodeData = nodes.find((n) => n.id === selectedNode) ?? null;

    return (
        <main className="min-h-screen bg-gray-50">
            <div className="mx-auto max-w-6xl p-6">
                {/* Header */}
                <div className="mb-6 flex items-center justify-between">
                    <div>
                        <h1 className="text-2xl font-semibold">TigerBeetle Cluster</h1>
                        <p className="text-sm text-gray-500">{onlineCount}/{nodes.length} nodes online</p>
                    </div>
                    <button onClick={handleLogout}
                            className="rounded-md border border-gray-300 bg-white px-4 py-2 text-sm hover:bg-gray-50">
                        Sign out
                    </button>
                </div>

                {cluster.isLoading && (
                    <div className="rounded-lg border border-gray-200 bg-white p-8 text-center">
                        <p className="text-sm text-gray-500">Connecting to cluster nodes...</p>
                    </div>
                )}

                {/* Node grid */}
                <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                    {nodes.map((node) => (
                        <button
                            key={node.id}
                            onClick={() => setSelectedNode(node.id)}
                            className={`rounded-lg border bg-white p-4 text-left transition-shadow hover:shadow-md focus:outline-none ${
                                node.online ? "border-gray-200" : "border-red-200 bg-red-50/50"
                            }`}
                        >
                            <div className="mb-3 flex items-center justify-between">
                                <h3 className="font-mono text-sm font-semibold">{node.id}</h3>
                                {node.online ? (
                                    <span className="flex items-center gap-1 text-xs text-green-600">
                                        <span className="inline-block h-2 w-2 rounded-full bg-green-500"/>Online
                                    </span>
                                ) : (
                                    <span className="flex items-center gap-1 text-xs text-red-600">
                                        <span className="inline-block h-2 w-2 rounded-full bg-red-500"/>Offline
                                    </span>
                                )}
                            </div>

                            {!node.online &&
                                <p className="text-xs text-red-600">Cannot reach {node.host}:{node.port}</p>}

                            {node.online && node.status && (
                                <div className="space-y-1.5 text-xs">
                                    <div className="flex justify-between">
                                        <span className="text-gray-500">Process</span>
                                        <ProcessStateBadge
                                            state={node.status.process?.state || "PROCESS_STATE_UNKNOWN"}/>
                                    </div>
                                    <div className="flex justify-between">
                                        <span className="text-gray-500">Address</span>
                                        <span className="font-mono">:{node.status.process?.address || "—"}</span>
                                    </div>
                                    <div className="flex justify-between">
                                        <span className="text-gray-500">Uptime</span>
                                        <span>{formatUptime(node.status.uptime_seconds)}</span>
                                    </div>
                                    <div className="flex justify-between">
                                        <span className="text-gray-500">Backups</span>
                                        <span
                                            className={node.status.backup?.enabled ? "font-medium text-green-600" : "text-gray-400"}>
                                            {node.status.backup?.enabled ? `On · ${node.status.backup.cron_schedule}` : "Off"}
                                        </span>
                                    </div>
                                    <p className="mt-2 text-center text-gray-400">Click to manage →</p>
                                </div>
                            )}
                        </button>
                    ))}
                </div>

                <div className="mt-6 rounded-lg border border-amber-200 bg-amber-50 p-3">
                    <p className="text-xs text-amber-900">
                        <strong>Timezone:</strong> All cron schedules run in UTC. Convert your local time accordingly.
                    </p>
                </div>
            </div>

            {/* Node detail drawer */}
            {selectedNode && selectedNodeData && (
                <NodeDrawer
                    node={selectedNodeData}
                    onClose={() => setSelectedNode(null)}
                    onRefresh={() => cluster.refetch()}
                />
            )}
        </main>
    );
}
