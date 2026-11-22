import { api } from '../api';
import { showChoice, showPrompt } from './overlays';
import { errMsg } from '../utils';

/** UI creation never replaces a shared skill without an explicit choice.
 *  `onDeclined` hears a Cancel at the clash choice, so a form can say why
 *  nothing was saved (P1-57). */
export async function createSkillWithChoice(name: string, description: string, body: string, current: () => boolean, onDeclined?: () => void) {
  while (current()) {
    try { return await api.writeUserSkill(name, description, body, true); }
    catch (e) { if (!errMsg(e).includes('SKILL_EXISTS:')) throw e; }
    if (!current()) return null;
    const choice = await showChoice({
      title: `Replace the skill /${name}?`, body: "History can't undo this. The existing SKILL.md will be replaced.",
      confirmLabel: 'Replace', alternateLabel: 'Save under a new name', danger: true,
    });
    if (!current()) return null;
    if (choice === 'cancel') { onDeclined?.(); return null; }
    if (choice === 'confirm') return api.writeUserSkill(name, description, body, false);
    const next = await showPrompt({ title: 'Save under a new name', confirmLabel: 'Save skill', input: {label: 'Skill name', value: name, maxLength: 100} });
    if (!next) return null;
    name = next;
  }
  return null;
}
