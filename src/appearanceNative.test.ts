import { expect, it } from 'vitest';
import source from '../src-tauri/src/commands.rs?raw';

it('glass polarity sets the native window theme before applying vibrancy, without probing a window', () => {
  const command = source.split('pub fn set_window_vibrancy(')[1].split('\n///')[0];
  expect(command).toMatch(/set_theme\(if enabled\s*\{\s*Some\(if dark\s*\{\s*tauri::Theme::Dark/);
  expect(command).toMatch(/\}\)\s*\} else \{\s*None\s*\}\)/);
  expect(command.indexOf('set_theme')).toBeLessThan(command.indexOf('apply_vibrancy(&w'));
});
