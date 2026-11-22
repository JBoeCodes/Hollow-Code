/** Embedded-agent conversation state and controls. */
export {
  chatPanelOpen, setChatPanelOpen, toggleChatPanel, closeChatPanel,
  chatFocusRequest, requestChatFocus, focusChatPanel, chatTurns, setChatTurns,
  chatSessionId, chatCwd, chatTurnUsage, chatBusy, chatInput, setChatInput,
  chatSessionLoading, setChatSessionLoading, chatConversationVersion,
  chatAttachments, setChatAttachments, nextChatAttachmentId,
  attachSharedPage,
  chatAgent, setChatAgent,
  chatModel, setChatModel, chatPermissionMode, setChatPermissionMode,
  setChatPermissionModePersistent, confirmPermissionModeChange,
  fileClick,
  chatArtifacts, chatPreviewOpen, chatPreviewPath, chatPreviewNonce,
  openChatPreview, closeChatPreview, rebuildChatArtifacts, openFileInSidePane, vaultRoot,
  showChatVisual, chatVisuals,
  appendToChatDraft,
  sidePaneWidth, setSidePaneWidth, persistSidePaneWidth, SIDE_PANE_MIN, SIDE_PANE_CHAT_MIN,
  pendingApprovals, approvalsResponding, respondToApproval,
  chatQueuedTurn, isVisiblyQueued, isPreparingTurn, turnIsQueuedTurn, withdrawQueuedChatTurn,
  ensureQueuedTurnVisible, reconcileQueuedChatRows, chatStopKey,
  AGENT_LABELS, agentSettings, agentDescriptors, currentAgentCapabilities, refreshAgentSettings, AGENT_DEFAULT_MODEL, KNOWN_AGENTS,
  PERM_MODE_ORDER, PERM_MODE_DANGER, PERM_MODE_LABELS, PERM_MODE_DESC,
} from './runtime';
export type {
  ChatAgent, ChatPermissionMode, ChatTurn, ChatToolUse,
  ChatToolStatus, ChatArtifact, ChatArtifactKind, ChatAttachment, ChatQueuedTurn,
} from './runtime';
