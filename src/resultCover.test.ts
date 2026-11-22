import { expect, it } from 'vitest';
import { resultCover } from './resultCover';

it('uses listed file metadata without claiming a preview', () => {
  expect(resultCover({ name: 'outputs/report.final.PDF', kindLabel: 'PDF', size: 2048 })).toEqual({ format: 'PDF', size: '2 KB' });
  expect(resultCover({ name: 'outputs/table.csv', kindLabel: 'Table', size: 0 })).toEqual({ format: 'CSV', size: '0 B' });
  expect(resultCover({ name: 'outputs/README', kindLabel: 'Document', size: -1 })).toEqual({ format: 'Document', size: null });
  expect(resultCover({ name: 'outputs/.hidden', kindLabel: 'File', size: NaN })).toEqual({ format: 'File', size: null });
  expect(resultCover({ name: 'outputs/file.unreasonablylongextension', kindLabel: 'File', size: Infinity })).toEqual({ format: 'File', size: null });
});
