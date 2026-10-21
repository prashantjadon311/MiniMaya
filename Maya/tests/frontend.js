#!/usr/bin/env node
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {spawnSync}=require('node:child_process');
const {pathToFileURL}=require('node:url');

const root=path.resolve(__dirname,'..');
const read=relative=>fs.readFileSync(path.join(root,relative),'utf8');
const scripts=['dom','api','voice','chat','tasks','activity','workspace','app'];
const html=read('web/index.html');
assert.deepEqual([...html.matchAll(/data-tab="([^"]+)"/g)].map(match=>match[1]),['chat','tasks','activity','workspace']);
assert.equal([...html.matchAll(/data-panel/g)].length,4);
assert.match(html,/id="approval-dialog"/);

for(const name of scripts){
  const relative=`web/js/${name}.js`;
  const source=read(relative);
  assert.doesNotMatch(source,/\.innerHTML\s*=/,`${relative} must use safe DOM construction`);
  assert.doesNotMatch(source,/localStorage|sessionStorage|new EventSource/,`${relative} must keep credentials in memory and use fetch streaming`);
  const checked=spawnSync(process.execPath,['--check',path.join(root,relative)],{encoding:'utf8'});
  assert.equal(checked.status,0,checked.stderr);
}

const api=read('web/js/api.js');
assert.match(api,/history\.replaceState/);
assert.match(api,/Authorization:`Bearer/);
assert.match(api,/X-CSRF-Token/);
assert.match(api,/Idempotency-Key/);
assert.match(api,/response\.body\.getReader/);
assert.doesNotMatch(api,/[?&](access_token|token)=/);

const app=read('web/js/app.js');
assert.match(app,/new CustomEvent\('maya:event'/,'Authenticated SSE events must reach the active UI');

const workspace=read('web/js/workspace.js');
for(const heading of ['Assistant identity','Models and providers','API credentials','Voice settings','Projects and coding tools','Reminders, security and resources'])assert.match(workspace,new RegExp(heading));
for(const route of ['/settings','/providers','/models','/model-roles','/projects','/tools','/commands','/capabilities','/system/resources','/backups'])assert.match(workspace,new RegExp(route.replace('/','\\/')));

for(const protocol of ['responses','chat_completions','gemini_generate_content','anthropic_messages'])assert.match(workspace,new RegExp(protocol));
assert.match(workspace,/\/jobs/,'Workspace must create persisted coding jobs');
assert.doesNotMatch(workspace,/bengali|spanish|french|german|japanese|other|friendly|formal/);
assert.match(workspace,/\['english','hindi','hinglish'\]/);
assert.match(workspace,/\['concise','standard','detailed'\]/);
assert.doesNotMatch(workspace,/context_limit_bytes:1048576/);
for(const field of ['input_device','output_device','work_days','daily_review_time','report_delivery','storage_cap_bytes'])assert.match(workspace,new RegExp(field));

const activity=read('web/js/activity.js');
assert.match(activity,/\/approval-requests/,'Activity must request immutable-plan implementation approval');
assert.match(activity,/project_edit/,'Activity must bind implementation approval to project_edit');
for(const recovery of ['inspect','continue_verification','replan','restore'])assert.match(activity,new RegExp(recovery));

const voice=read('web/js/voice.js');
assert.match(voice,/\/actions/,'Cloud voice must create a persisted approval operation');
assert.match(voice,/voice_upload/);
assert.match(voice,/payload_digest/);
assert.match(voice,/approval_id/);

const chat=read('web/js/chat.js');
assert.match(chat,/startCapture/,'Chat must expose user-activated voice capture');
assert.match(chat,/stopCapture/,'Chat must stop capture and consume the transcript');
assert.match(chat,/pendingJobs/,'Chat must track asynchronous response jobs');
assert.match(chat,/completed_with_issues/,'Chat must handle terminal response states');
assert.match(chat,/Open Activity for recovery details/,'Chat must surface recovery guidance');

const taskModule=pathToFileURL(path.join(root,'web/js/tasks.js')).href;
const taskUnit=spawnSync(process.execPath,['--input-type=module','-e',`
  import assert from 'node:assert/strict';
  import {isoWeek,dueAtRfc3339,progressPatch} from ${JSON.stringify(taskModule)};
  assert.equal(isoWeek(new Date('2021-01-01T12:00:00Z')),'2020-W53');
  assert.equal(dueAtRfc3339(''),null);
  assert.match(dueAtRfc3339('2026-10-21T15:30'),/^2026-10-21T/);
  assert.deepEqual(progressPatch({id:'task',revision:4},40,'in_progress'),{revision:4,status:'in_progress',progress_pct:40,progress_source:'manual'});
`],{encoding:'utf8'});
assert.equal(taskUnit.status,0,taskUnit.stderr);

const workspaceModule=pathToFileURL(path.join(root,'web/js/workspace.js')).href;
const workspaceUnit=spawnSync(process.execPath,['--input-type=module','-e',`
  import assert from 'node:assert/strict';
  import {commandArguments} from ${JSON.stringify(workspaceModule)};
  assert.deepEqual(commandArguments('["test","--locked","name with space"]'),['test','--locked','name with space']);
  assert.throws(()=>commandArguments('{"command":"test"}'));
  assert.throws(()=>commandArguments('["ok",7]'));
`],{encoding:'utf8'});
assert.equal(workspaceUnit.status,0,workspaceUnit.stderr);

console.log('frontend contract checks: PASS');
