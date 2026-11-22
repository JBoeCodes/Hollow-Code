import { expect, it } from 'vitest';
import { mostRecentProject, projectChoices } from './projectDestination';
import type { WorkWorkspace, WorkspaceSessions } from './workspaceTypes';
const project=(id:string,status='active')=>({id,title:id,status,modified:'2099-01-01',cwd:'/vault/'+id,path:'workspaces/'+id}) as WorkWorkspace;
const group=(time:string,turnCount=1)=>({sessions:[{lastActivityAt:time,turnCount}]}) as WorkspaceSessions;
it('defaults to recorded work, not alphabetic order, pins, brief mtime or a zero-turn coordinator',()=>{
 const list=[project('A newly created'),project('Z worked'),project('B older')];
 const groups={'A newly created':group('2099-01-01',0),'Z worked':group('2026-10-26'),'B older':group('2026-10-25')};
 expect(mostRecentProject(list,groups)).toBe('Z worked');
 expect(mostRecentProject(list,{})).toBe('');
});
it('skips deleted, archived, merged, built-in and missing-worktree projects',()=>{
 const list=[project('old','archived'),{...project('merged'),mergedInto:'target'},{...project('missing'),build:{exists:false}},project('main'),project('eligible') ] as WorkWorkspace[];
 expect(projectChoices(list,{}).map(c=>c.project.id)).toEqual(['eligible']);
 expect(mostRecentProject(list,{old:group('2099-01-01'),eligible:group('2026-10-26')})).toBe('eligible');
});
it('excludes the source project from both choices and the recent default',()=>{
 const list=[project('current'),project('other')];
 const groups={current:group('2026-10-27'),other:group('2026-10-26')};
 expect(projectChoices(list,groups,'current').map(c=>c.project.id)).toEqual(['other']);
 expect(mostRecentProject(list,groups,'current')).toBe('other');
 expect(mostRecentProject([project('current')],groups,'current')).toBe('');
});
