// Settings' capability-doctor controller (plan completion L4, X18: extracted
// from pages/Settings.tsx): the diagnostics, backup-health and installed-
// runtime reads, the doctor's checks and preferences, its live probes and
// fixes, and clearing debug logs or saving a diagnostics bundle. Every read
// is wrapped (`preserveDuringRefetch`) so running the checks never blanks
// the console it reports in. Called once by the Settings page.
import { backupManagedElsewhere } from '../../store/prefs';
import { createResource, createSignal, createMemo } from 'solid-js';
import { plaudStatus, connectPlaud, checkPlaudStatus } from '../../store/connectors';
import { saveDiagnosticsBundle } from '../../store/diagnostics';
import { flashFooter, trackTask } from '../../store';
import { api } from '../../api';
import { isFirstLoad, preserveDuringRefetch } from '../../stableResource';
import type { BackupHealth, Diagnostics } from '../../types';
import type { TrackerStatus } from '../../generated/ipc';
import { trackerStatus } from '../../trackerApi';
import { CAPABILITY_HOMES, EMPTY_CAPABILITY_DOCTOR_PREFS, buildCapabilityChecks, markCapabilityVerified, readCapabilityDoctorPrefs, setCapabilityDeclined, summarizeCapabilities, type CapabilityCheck, type CapabilityDoctorPrefs, type DeclineableCapabilityId, type VerifiableCapabilityId } from '../../capabilityDoctor';

/** "1 need attention" is wrong; the count reaches 1 routinely. */
export function needAttention(n: number): string {
  return `${n} ${n === 1 ? 'needs' : 'need'} attention`;
}

export interface CapabilityDoctorDeps {
  openSection: (id: string) => void;
  /** Open a section on one row (Settings' `jumpTo`): a fix lands on the row that fixes it. */
  openControl: (id: string, label?: string) => void;
}

export function createCapabilityDoctor(deps: CapabilityDoctorDeps) {
  const { openControl } = deps;
  const [_diag, { refetch: refetchDiag }] = createResource<Diagnostics>(() => api.diagnostics());
  // The task tracker (A14): read here so the doctor and Service health have it
  // in every section; Settings' Task tracker row writes its fresher reads back.
  const [tracker, { mutate: setTrackerStatus, refetch: refetchTracker }] = createResource<TrackerStatus | null>(() => trackerStatus().catch(() => null));
  const diag = preserveDuringRefetch(_diag);

  const backupManaged = backupManagedElsewhere;
  const [_backupHealth, { refetch: refetchBackupHealth }] = createResource<BackupHealth, string>(
    () => backupManaged() ? 'managed' : 'automatic',
    mode => api.backupHealth(mode === 'managed'),
  );
  const backupHealth = preserveDuringRefetch(_backupHealth);
  // Read installed runtimes directly so diagnostics reflect this computer.
  const [_runtimeAgents, { refetch: refetchRuntimeAgents }] = createResource<string[]>(() => api.agentAvailable());
  const runtimeAgents = preserveDuringRefetch(_runtimeAgents);

  const initialDoctorPrefs: CapabilityDoctorPrefs = typeof localStorage === 'undefined'
    ? { ...EMPTY_CAPABILITY_DOCTOR_PREFS, declined: {}, lastVerified: {} }
    : readCapabilityDoctorPrefs(localStorage);
  const [doctorPrefs, setDoctorPrefs] = createSignal<CapabilityDoctorPrefs>(initialDoctorPrefs);
  const [doctorProbeErrors, setDoctorProbeErrors] = createSignal<Partial<Record<VerifiableCapabilityId, string>>>({});
  const [doctorRunning, setDoctorRunning] = createSignal(false);
  // Friendly error message when the Tauri IPC isn't available (e.g.
  // browser-mode dev smoke, or if `diagnostics` ever throws on the
  // Rust side). Without this the entire Paths / Connector / Debug
  // sections would render empty with no explanation.
  const diagError = () => {
    const e = diag.error;
    if (!e) return null;
    return typeof e === 'string' ? e : (e as { message?: string }).message || String(e);
  };
  const [clearing, setClearing] = createSignal(false);
  const [savingDiag, setSavingDiag] = createSignal(false);
  async function saveDiag() {
    if (savingDiag()) return;
    setSavingDiag(true);
    try { await saveDiagnosticsBundle(); } finally { setSavingDiag(false); }
  }

  async function clearLogs() {
    if (clearing()) return;
    setClearing(true);
    try {
      const n = await api.clearDebugLogs();
      flashFooter(n > 0 ? `Cleared ${n} debug log file${n === 1 ? '' : 's'}` : 'No debug logs to clear', 2200);
      await refetchDiag();
    } catch (e: any) {
      flashFooter(`× clear logs failed: ${e?.message || e}`, 3500);
    } finally {
      setClearing(false);
    }
  }


  const doctorChecks = createMemo(() => buildCapabilityChecks({
    vaultExists: diag()?.vaultExists ?? null,
    diagnosticsFailed: !!diagError(),
    // isFirstLoad, not `loading`: loading is also true while refreshing, so
    // re-running the checks would flip a live row to unknown and back.
    agents: isFirstLoad(runtimeAgents) ? null : (runtimeAgents() ?? null),
    agentProbeFailed: !!runtimeAgents.error,
    plaudStatus: plaudStatus(),
    plaudError: !!doctorProbeErrors().plaud,
    plaudLastSuccess: null,
    tracker: isFirstLoad(tracker) ? null : (tracker() ?? null),
    context: diag()?.contextHealth ?? undefined,
    backup: backupHealth() ?? null,
    backupFailed: !!backupHealth.error,
    prefs: doctorPrefs(),
  }));
  const doctorSummary = createMemo(() => summarizeCapabilities(doctorChecks()));

  function saveDecline(id: DeclineableCapabilityId, declined: boolean) {
    if (typeof localStorage === 'undefined') {
      setDoctorPrefs(prev => {
        const next = { ...prev, declined: { ...prev.declined } };
        if (declined) next.declined[id] = new Date().toISOString();
        else delete next.declined[id];
        return next;
      });
      return;
    }
    setDoctorPrefs(setCapabilityDeclined(localStorage, id, declined));
  }

  function saveVerified(id: VerifiableCapabilityId) {
    if (typeof localStorage === 'undefined') {
      setDoctorPrefs(prev => ({
        ...prev,
        declined: Object.fromEntries(Object.entries(prev.declined).filter(([key]) => key !== id)),
        lastVerified: { ...prev.lastVerified, [id]: new Date().toISOString() },
      }));
      return;
    }
    setDoctorPrefs(markCapabilityVerified(localStorage, id));
  }

  function clearDoctorProbeError(id: VerifiableCapabilityId) {
    setDoctorProbeErrors(prev => {
      const next = { ...prev };
      delete next[id];
      return next;
    });
  }

  async function probeCapability(id: VerifiableCapabilityId): Promise<void> {
    try {
      const ok = await trackTask(
        'Verify Plaud',
        async () => api.plaudPing(),
        { detail: 'Capability doctor live probe', onSuccessResult: value => value ? 'verified' : 'failed' },
      );
      if (!ok) throw new Error('The capability did not pass its live probe.');
      clearDoctorProbeError(id);
      saveVerified(id);
      if (id === 'plaud') await checkPlaudStatus();
      const final = doctorChecks().find(item => item.id === id);
      if (final?.state === 'live') {
        flashFooter(`${final.label} verified`, 2200);
      } else {
        flashFooter(`${final?.label ?? id} is reachable, but an existing sync error still needs attention`, 3200);
        openControl(CAPABILITY_HOMES.plaud.section, CAPABILITY_HOMES.plaud.control);
      }
    } catch (e: any) {
      const message = e?.message || String(e);
      setDoctorProbeErrors(prev => ({ ...prev, [id]: message }));
      flashFooter(`× capability check failed: ${message}`, 3500);
      openControl(CAPABILITY_HOMES.plaud.section, CAPABILITY_HOMES.plaud.control);
    }
  }

  async function runDoctorChecks() {
    if (doctorRunning()) return;
    setDoctorRunning(true);
    setDoctorProbeErrors({});
    try {
      const result = await trackTask(
        'Run capability doctor',
        async () => {
          await Promise.allSettled([
            refetchDiag(),
            refetchRuntimeAgents(),
            refetchTracker(),
            refetchBackupHealth(),
            checkPlaudStatus(),
          ]);

          const ids: VerifiableCapabilityId[] = ['plaud'];
          const active = ids.filter(id => !doctorPrefs().declined[id]);
          const settled = await Promise.allSettled(active.map(async id => {
            const ok = await api.plaudPing();
            if (!ok) throw new Error('live probe returned false');
            return id;
          }));

          const failures: Partial<Record<VerifiableCapabilityId, string>> = {};
          settled.forEach((entry, index) => {
            const id = active[index];
            if (entry.status === 'fulfilled') {
              saveVerified(id);
            } else {
              failures[id] = entry.reason?.message || String(entry.reason);
            }
          });
          setDoctorProbeErrors(failures);
          return summarizeCapabilities(doctorChecks());
        },
        {
          detail: 'Runtime, connector, and vault checks',
          onSuccessResult: value => `${value.live} working · ${needAttention(value.needsAttention)}`,
          onSuccessStatus: value => value.needsAttention > 0 ? 'warning' : 'success',
        },
      );
      flashFooter(`Capability doctor: ${result.live} working · ${needAttention(result.needsAttention)}`, 2600);
    } catch (e: any) {
      flashFooter(`× capability doctor failed: ${e?.message || e}`, 3500);
    } finally {
      setDoctorRunning(false);
    }
  }

  async function capabilityAction(item: CapabilityCheck) {
    if (item.state === 'declined' && item.declinable) {
      saveDecline(item.id as DeclineableCapabilityId, false);
      return;
    }
    if (item.id === 'agent-runtime') {
      await refetchRuntimeAgents();
      openControl(item.fixSection, item.fixControl);
      return;
    }
    if (item.id === 'backup') {
      await refetchBackupHealth();
      openControl(item.fixSection, item.fixControl);
      return;
    }
    if (item.id === 'plaud') {
      if (item.state === 'broken' && plaudStatus() === 'needs-auth') connectPlaud();
      else if (item.state === 'stale' || (item.state === 'broken' && !!doctorProbeErrors().plaud)) await probeCapability('plaud');
      else openControl(item.fixSection, item.fixControl);
      return;
    }
    openControl(item.fixSection, item.fixControl);
  }

  return { tracker, setTrackerStatus, diag, refetchDiag, diagError, backupManaged, backupHealth, refetchBackupHealth, runtimeAgents, refetchRuntimeAgents, doctorPrefs, doctorProbeErrors, doctorRunning, doctorChecks, doctorSummary, saveDecline, saveVerified, probeCapability, runDoctorChecks, capabilityAction, clearing, clearLogs, savingDiag, saveDiag };
}
