import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  view: vi.fn(), accept: vi.fn(), acceptAll: vi.fn(), answer: vi.fn(), dismiss: vi.fn(), refetch: vi.fn(), flash: vi.fn(),
}));
vi.mock('../api', () => ({ api: {
  profileBuilderView: mocks.view, profileBuilderAccept: mocks.accept, profileBuilderAcceptAll: mocks.acceptAll,
  profileBuilderAnswer: mocks.answer, profileBuilderDismiss: mocks.dismiss, profileBuilderScan: vi.fn(),
} }));
vi.mock('../nativeEvents', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('./runtime', () => ({ refetchOperatorProfile: mocks.refetch }));
vi.mock('./snackbars', () => ({ flashFooter: mocks.flash }));
vi.mock('./activityDock', () => ({ trackTask: (_name: string, fn: () => unknown) => fn() }));

import profileSuggestionsSource from '../components/ProfileSuggestions.tsx?raw';
import aboutYouSource from '../settings/AboutYouPanel.tsx?raw';
import settingsSource from '../pages/Settings.tsx?raw';
import learningsSource from '../components/Learnings.tsx?raw';
import { acceptProfileSuggestion, actProfile, loadProfileBuilder, profileBusy, profileView, profileWaitingCount } from './profileBuilder';

const suggestion = (id: string) => ({ id, file: 'Identity.md', section: 'Name', label: null, text: `fact ${id}`, source: 'claude-memory' });
const view = (ids: string[], questions = 0) => ({
  profile: [], sources: [], suggestions: ids.map(suggestion),
  questions: Array.from({ length: questions }, (_, i) => ({ id: `q${i}`, question: '?', label: 'l', why: null, file: 'Identity.md', section: null })),
  scannedAt: null, scanning: false, autoLearn: false, lastError: null,
});

beforeEach(() => { Object.values(mocks).forEach(m => m.mockReset()); });

describe('profile suggestions in Library, Learnings (Settings PR 4)', () => {
  it('accepting one writes the same note through the same call, with the edit when there is one', async () => {
    mocks.view.mockResolvedValue(view(['a', 'b']));
    await loadProfileBuilder();
    expect(profileWaitingCount()).toBe(2);
    mocks.accept.mockResolvedValue(view(['b']));
    await acceptProfileSuggestion('a', 'fact a, edited');
    expect(mocks.accept).toHaveBeenCalledTimes(1);
    expect(mocks.accept).toHaveBeenCalledWith('a', 'fact a, edited');
    expect(profileView()?.suggestions.map(s => s.id)).toEqual(['b']);
    expect(mocks.refetch).toHaveBeenCalled();
    expect(mocks.flash).toHaveBeenCalledWith('Added to your profile', 2200);
    expect(profileBusy()).toBeNull();
  });

  it('accepts as written when the person did not edit', async () => {
    mocks.accept.mockResolvedValue(view([]));
    await acceptProfileSuggestion('a');
    expect(mocks.accept).toHaveBeenCalledWith('a', undefined);
  });

  it('counts suggestions and questions for the Personalization line, and does one action at a time', async () => {
    mocks.view.mockResolvedValue(view(['a'], 2));
    await loadProfileBuilder();
    expect(profileWaitingCount()).toBe(3);
    let release!: () => void;
    mocks.answer.mockImplementation(() => new Promise(done => { release = () => done(view(['a'], 1)); }));
    const first = actProfile('q0', () => mocks.answer());
    await actProfile('q1', () => mocks.dismiss());
    expect(mocks.dismiss).not.toHaveBeenCalled();
    release();
    await first;
    expect(profileWaitingCount()).toBe(2);
  });

  it('shows a refused call in the footer and keeps the view', async () => {
    mocks.view.mockResolvedValue(view(['a']));
    await loadProfileBuilder();
    mocks.accept.mockRejectedValue(new Error('note is read-only'));
    await acceptProfileSuggestion('a');
    expect(mocks.flash).toHaveBeenCalledWith(expect.stringContaining('note is read-only'), 3500);
    expect(profileView()?.suggestions).toHaveLength(1);
  });

  it('has one home: Learnings shows the suggestions, Settings keeps only the scan, notes and one line', () => {
    expect(learningsSource).toContain('<ProfileSuggestions />');
    expect(profileSuggestionsSource).toContain('acceptProfileSuggestion(id, text)');
    expect(profileSuggestionsSource).toContain('api.profileBuilderAcceptAll()');
    expect(profileSuggestionsSource).toContain('Review all');
    for (const gone of ['Ready to add', 'about-you-focus', 'Review all', 'profileBuilderAccept', 'profileBuilderAnswer']) expect(aboutYouSource, gone).not.toContain(gone);
    expect(aboutYouSource).toContain('Waiting for you');
    expect(aboutYouSource).toContain('openLibraryLearnings');
    expect(settingsSource).not.toContain('ProfileSuggestions');
  });
});
