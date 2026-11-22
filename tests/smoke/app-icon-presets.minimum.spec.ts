import { expect, test } from './fixtures/test';
import { openSettingsIcons } from './choose';
test('app presets fit and remain keyboard reachable at minimum size',async({page},info)=>{
  await page.goto('/?mock=1');
  await page.getByRole('navigation',{name:'Shared tools'}).getByRole('button',{name:'Settings',exact:true}).click();
  await page.getByRole('combobox',{name:'Settings section'}).click();
  await page.locator('.hs-pop').getByRole('option',{name:/^General/}).click();
  await openSettingsIcons(page);
  const row=page.getByRole('region',{name:'App icon',exact:true});
  await expect(row.getByRole('status')).toHaveText('Selected: Fold (default)');
  for(const name of ['Fold (default)','Threshold','Cut Glass','Aurora','Monolith']) {
    const button=row.getByRole('button',{name,exact:true});
    await button.focus();await page.keyboard.press('Enter');
    await expect(button).toHaveAttribute('aria-pressed','true');await expect(button).toBeFocused();
    const bounds=await button.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x+bounds!.width).toBeLessThanOrEqual(page.viewportSize()!.width);
  }
  await row.screenshot({path:info.outputPath('picker-minimum.png')});
});
