import { describe, expect, it } from 'vitest';
import { usageSeries, stackedPaths, budgetGauge, coverageWords, agentAmount } from './usageInstruments';
import type { UsageBucket, UsageDay } from './types';
const bucket = (key: string, fields: Partial<UsageBucket> = {}): UsageBucket => ({ key, label:key, runs:1, metered:1, partial:0,unknown:0,notStarted:0,inProgress:0,unpriced:0,inputTokens:0,cachedInputTokens:0,outputTokens:0,reportedCostUsd:null,estimatedCostUsd:null,...fields });
const day = (date: string, byAgent: UsageBucket[]): UsageDay => ({date,total:bucket('total'),byAgent,nightByAgent:[]});
describe('Usage instruments',()=>{
 it('keeps estimated and reported separate, and missing prices null',()=>{
  const days=[day('2026-10-01',[bucket('claude',{reportedCostUsd:2,estimatedCostUsd:8}),bucket('codex',{unpriced:1})])];
  expect(usageSeries(days,'reportedCostUsd').map(s=>s.values)).toEqual([[2],[null]]);
  expect(usageSeries(days,'estimatedCostUsd').map(s=>s.values)).toEqual([[8],[null]]);
 });
 it('does not bridge unrecorded calendar days or fabricate a free run',()=>{
  const days=[day('2026-10-01',[bucket('claude',{reportedCostUsd:2})]),day('2026-10-03',[bucket('claude',{reportedCostUsd:4})])];
  const result=stackedPaths(days,'reportedCostUsd');
  expect(result.layers[0].paths).toHaveLength(2);
  expect(result.max).toBe(4);
  expect(result.layers[0].paths.join('')).not.toMatch(/NaN|Infinity/);
 });
 it('stacks agents to the daily sum and preserves a singleton',()=>{
  const days=[day('2026-10-01',[bucket('claude',{estimatedCostUsd:2}),bucket('codex',{estimatedCostUsd:3})])];
  expect(stackedPaths(days,'estimatedCostUsd').max).toBe(5);
  expect(stackedPaths(days,'estimatedCostUsd').layers.every(l=>l.paths.length===1)).toBe(true);
 });
 it('never makes an unknown gauge empty; clamps graphic only',()=>{
  expect(budgetGauge(null)).toBeNull();expect(budgetGauge(NaN)).toBeNull();
  expect(budgetGauge(150)?.fraction).toBe(1);expect(budgetGauge(150)?.label).toBe('150%');
  expect(budgetGauge(0)?.fraction).toBe(0);
 });
 it('discloses all coverage classes',()=>{
  expect(coverageWords(bucket('x',{partial:1,unknown:2,unpriced:3,inProgress:4,notStarted:5}))).toContain('1 partial');
  expect(coverageWords(bucket('x',{unknown:2}))).toContain('2 unknown');
  expect(coverageWords(bucket('x',{gpuTime:3}))).toBe('3 billed by GPU time');
 });
 it('says the self-hosted agent split is billed by GPU time (Q8 second review)',()=>{
  expect(agentAmount(bucket('self-hosted:qwen3.8-27b',{gpuTime:2}),'reportedCostUsd')).toBe('Billed by GPU time');
  expect(agentAmount(bucket('claude'),'reportedCostUsd')).toBe('No amount recorded');
  expect(agentAmount(bucket('claude',{reportedCostUsd:0.42}),'reportedCostUsd')).toBe('$0.42');
 });
});
