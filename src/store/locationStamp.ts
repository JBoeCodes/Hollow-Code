// Where the person is, as one comparable string (A05): the page, the open
// workspace, and Your work's and Review's views. The command palette reads it
// before and after running a row; a row that leaves it unchanged (a toggle, a
// copy) gives focus back to what opened the palette, one that changes it
// leaves focus in the new view. Read from the existing signals, so no setter
// has to bump anything. Its own module: destinations.ts can't import runtime
// or workspaces (workspaces imports destinations), and only the palette's lazy
// chunk imports this, so it stays out of the launch chunk.
import { shownPage } from './runtime';
import { workspaceId } from './workspaces';
import { reviewView, workView } from './destinations';

// Library → Conversations (`chat` in shownPage) is apart from Library (S4).
// Today's Day / Week range left with the Today screen (overnight 11-08 S3).
export const locationStamp = (): string => `${shownPage()}|${workspaceId()}|${workView()}|${reviewView()}`;
