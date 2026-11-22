// Generated from src-tauri/src/chat_event.rs by src-tauri/src/ts_bindings.rs (ts-rs).
// The `chat-event` payload schema (AG2). Do not edit by hand: change the Rust
// type, then from src-tauri/ run
//   HOLLOW_REGENERATE_IPC=1 cargo test --lib ts_bindings
// Nested objects use the frontend's own types, imported here.
import type { EvidenceReceiptWire, EvidenceRecallItemWire } from '../executionReceipt';
import type { PauseReason } from '../chatStop';
import type { SessionResetReason } from '../chatContinue';
import type { RemoteRunEvent } from '../remoteRunNotice';
import type { SteerSupportRow, SteerWire } from '../chatSteer';
import type { ChatRecallTrace } from '../store/chatRuntime';
import type { BudgetNotice, ChatInteraction, ProgressPhase, TurnUsage } from '../types';

/** A union member without its `type` (kept distributive over a union). */
type WithoutType<T> = T extends unknown ? Omit<T, 'type'> : never;

// hollow_lib::chat_event::AccountSwitchedEvent
export type AccountSwitchedEvent = {
  from: string,
  to: string,
  reason?: 'limit' | 'ceiling' | null,
  sessionKey?: string | null,
};

// hollow_lib::chat_event::ActivityEvent
export type ActivityEvent = {
  runId: string,
  sessionKey?: string | null,
  lastEventAt: string | null,
  progress: ProgressPhase | null,
};

// hollow_lib::chat_event::AgentContinuedEvent
export type AgentContinuedEvent = {
  from: string,
  to: string,
  prompt: string,
  sessionKey: string,
};

// hollow_lib::chat_event::AssistantTextEvent
export type AssistantTextEvent = {
  text: string,
  parentToolUseId?: string | null,
};

// hollow_lib::chat_event::BudgetWarningEvent
export type BudgetWarningEvent = {
  sessionKey: string,
  notice: BudgetNotice,
};

// hollow_lib::chat_event::CancelledEvent
export type CancelledEvent = {
  stop?: 'verified' | 'unconfirmed' | null,
  queued?: boolean,
  beforeSpawn?: boolean,
};

// hollow_lib::chat_event::ChatEventEnvelope
export type ChatEventEnvelope = {
  clientTurnId?: string | null,
  queueId?: number | null,
  conversationKey?: string | null,
} & ({ "type": "session" } & SessionEvent | { "type": "runStarted" } & RunStartedEvent | { "type": "assistantText" } & AssistantTextEvent | { "type": "assistantDelta" } & AssistantTextEvent | { "type": "turnBoundary" } & TurnBoundaryEvent | { "type": "toolUse" } & ToolUseEvent | { "type": "toolResult" } & ToolResultEvent | { "type": "toolImages" } & ToolImagesEvent | { "type": "evidenceReceipt" } & EvidenceReceiptEvent | { "type": "result" } & ResultEvent | { "type": "usage" } & UsageEvent | { "type": "activity" } & ActivityEvent | { "type": "budgetWarning" } & BudgetWarningEvent | { "type": "loginLow" } & SessionMessageEvent | { "type": "panel" } & PanelEvent | { "type": "interaction" } & InteractionEvent | { "type": "cancelled" } & CancelledEvent | { "type": "paused" } & PausedEvent | { "type": "sessionReset" } & SessionResetEvent | { "type": "accountSwitched" } & AccountSwitchedEvent | { "type": "remoteRun" } & WithoutType<RemoteRunEvent> | { "type": "error" } & ErrorEvent | { "type": "done" } & DoneEvent | { "type": "steer" } & SteerEvent | { "type": "heldSent" } & HeldSentEvent | { "type": "agentContinued" } & AgentContinuedEvent | { "type": "heldFailed" } & HeldFailedEvent);

// hollow_lib::chat_event::DoneEvent
export type DoneEvent = {
  sessionId: string | null,
  sessionKey?: string | null,
  recovered?: boolean,
};

// hollow_lib::chat_event::ErrorEvent
export type ErrorEvent = {
  message: string,
  resultKind?: string,
};

// hollow_lib::chat_event::EvidenceReceiptEvent
export type EvidenceReceiptEvent = {
  receipt: EvidenceReceiptWire,
};

// hollow_lib::chat_event::HeldFailedEvent
export type HeldFailedEvent = {
  heldId: string,
  error: string,
  prompt: string,
  sessionKey: string,
};

// hollow_lib::chat_event::HeldSentEvent
export type HeldSentEvent = {
  heldId: string,
  after: 'queue' | 'stop',
  sessionKey: string,
};

// hollow_lib::chat_event::InteractionEvent
export type InteractionEvent = {
  interaction: ChatInteraction,
  sessionKey?: string | null,
};

// hollow_lib::chat_event::PanelEvent
export type PanelEvent = {
  sessionKey: string,
  panel: string,
  snapshot: unknown,
};

// hollow_lib::chat_event::PausedEvent
export type PausedEvent = {
  sessionKey: string,
  note: string,
  at: number,
  reason?: PauseReason,
  resumeAt?: number | null,
};

// hollow_lib::chat_event::ResultEvent
export type ResultEvent = {
  result: string | null,
  costUsd: number | null,
  isError: boolean,
  subtype: string,
  sessionId: string | null,
  resultKind?: string,
};

// hollow_lib::chat_event::RunStartedEvent
export type RunStartedEvent = {
  runId: string,
  sessionKey?: string | null,
  recall: EvidenceRecallItemWire[],
  asked?: string | null,
  steerSupport?: SteerSupportRow | null,
  permissionMode?: string | null,
  rt?: ChatRecallTrace | null,
  unattended?: boolean | null,
  attended?: boolean | null,
};

// hollow_lib::chat_event::SessionEvent
export type SessionEvent = {
  sessionId: string,
};

// hollow_lib::chat_event::SessionMessageEvent
export type SessionMessageEvent = {
  sessionKey: string,
  message: string,
};

// hollow_lib::chat_event::SessionResetEvent
export type SessionResetEvent = {
  reason: SessionResetReason,
  agent: string,
  sessionKey?: string | null,
  changed?: Array<string>,
  away?: string,
};

// hollow_lib::chat_event::SteerEvent
export type SteerEvent = {
  sessionKey: string,
  steer: SteerWire,
};

// hollow_lib::chat_event::ToolImagesEvent
export type ToolImagesEvent = {
  id: string,
  images: Array<string>,
  parentToolUseId?: string | null,
};

// hollow_lib::chat_event::ToolResultEvent
export type ToolResultEvent = {
  id: string,
  content: string,
  isError: boolean,
  parentToolUseId?: string | null,
};

// hollow_lib::chat_event::ToolUseEvent
export type ToolUseEvent = {
  id?: string,
  name: string,
  input: unknown,
  parentToolUseId?: string | null,
};

// hollow_lib::chat_event::TurnBoundaryEvent
export type TurnBoundaryEvent = Record<symbol,
  never>;

// hollow_lib::chat_event::UsageEvent
export type UsageEvent = {
  usage: TurnUsage,
  sessionKey?: string | null,
};
