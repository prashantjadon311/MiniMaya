import {el,showError} from './dom.js';
import {request} from './api.js';

export async function save(group,patch){const current=await request('GET','/settings');return request('PATCH','/settings',{revision:current.revision,[group]:patch})}
export const probe=(kind,id)=>request('POST','/capabilities/probe',{kind,id});
const field=(label,value,type='text')=>{const input=el('input',{type,value:value??''});const row=el('label',{},label);row.append(input);return{row,input}};
const select=(label,value,choices)=>{const node=el('select');for(const choice of choices){const option=el('option',{value:choice.value??choice},choice.label??choice);option.selected=(choice.value??choice)===value;node.append(option)}const row=el('label',{},label);row.append(node);return{row,node}};
const check=(label,value)=>{const node=el('input',{type:'checkbox'});node.checked=Boolean(value);const row=el('label',{class:'row'},label);row.prepend(node);return{row,node}};
const formCard=title=>{const card=el('section',{class:'card'});card.append(el('h3',{},title));return card};
function submit(form,label,handler){const result=el('p',{class:'muted'}),button=el('button',{type:'submit'},label);form.append(button,result);form.addEventListener('submit',async event=>{event.preventDefault();button.disabled=true;try{await handler();result.textContent='Saved';result.className='muted'}catch(error){showError(result,error)}finally{button.disabled=false}})}

function identityCard(settings){
  const card=formCard('Assistant identity'),form=el('form');
  const name=field('Name',settings.identity.name),greeting=field('Greeting',settings.identity.greeting),language=select('Language',settings.identity.language,['english','hindi','hinglish']),tone=select('Tone',settings.identity.tone,['concise','standard','detailed']);
  form.append(name.row,greeting.row,language.row,tone.row);
  submit(form,'Save identity',async()=>{const updated=await save('identity',{name:name.input.value.trim(),greeting:greeting.input.value.trim(),language:language.node.value,tone:tone.node.value});document.getElementById('assistant-name').textContent=updated.identity.name});
  card.append(form);return card;
}

const providerProtocol=kind=>({openai_responses:'responses',openai_compatible_chat:'chat_completions',gemini:'gemini_generate_content',anthropic:'anthropic_messages'})[kind];
function providersCard(providers,models){
  const card=formCard('Models and providers'),summary=el('div');
  for(const provider of providers.items)summary.append(el('p',{class:'muted'},`${provider.label} · ${provider.kind} · credential ${provider.credential_status}`));
  for(const model of models.items)summary.append(el('p',{class:'muted'},`${model.label} · ${model.model_id} · ${model.enabled?'enabled':'disabled'}`));
  card.append(summary);
  const providerForm=el('form'),label=field('Provider label',''),endpoint=field('HTTPS endpoint','https://api.openai.com/v1'),kind=select('Provider kind','openai_responses',['openai_responses','openai_compatible_chat','gemini','anthropic']);
  providerForm.append(label.row,kind.row,endpoint.row);
  submit(providerForm,'Add provider',()=>request('POST','/providers',{label:label.input.value.trim(),kind:kind.node.value,protocol:providerProtocol(kind.node.value),endpoint:endpoint.input.value.trim(),local_endpoint_allowed:false,enabled:true,billing_mode:'api',timeout_ms:120000,owner_ack:true}));
  card.append(providerForm);
  const modelForm=el('form'),modelProvider=select('Provider',providers.items[0]?.id||'',providers.items.map(item=>({value:item.id,label:item.label}))),modelId=field('API model ID',''),modelLabel=field('Model label',''),contextLimit=field('Context limit bytes',32768,'number');
  contextLimit.input.min='1024';contextLimit.input.max='131072';contextLimit.input.step='1024';
  modelForm.append(modelProvider.row,modelId.row,modelLabel.row,contextLimit.row);
  submit(modelForm,'Add model',()=>request('POST','/models',{provider_id:modelProvider.node.value,model_id:modelId.input.value.trim(),label:modelLabel.input.value.trim(),enabled:true,capabilities:['text'],context_limit_bytes:Number(contextLimit.input.value)}));
  card.append(modelForm);return card;
}

function credentialCard(providers){
  const card=formCard('API credentials'),form=el('form'),provider=select('Provider',providers.items[0]?.id||'',providers.items.map(item=>({value:item.id,label:item.label}))),secret=field('API credential','','password'),mode=select('Storage','session',['session','keyring']);
  form.append(provider.row,secret.row,mode.row);
  submit(form,'Store credential',async()=>{try{await request('POST',`/providers/${encodeURIComponent(provider.node.value)}/secret`,{secret:secret.input.value,mode:mode.node.value})}finally{secret.input.value=''}});
  card.append(el('p',{class:'muted'},'Credentials are write-only and never returned by Maya.'),form);return card;
}

function voiceCard(settings,models){
  const card=formCard('Voice settings'),form=el('form'),stt=select('Speech input',settings.voice.stt,['disabled','openai_audio','compatible_audio']),tts=select('Speech output',settings.voice.tts,['disabled','browser','espeak_ng','openai_audio']),sttModel=field('STT model ID',settings.voice.stt_model_id||''),ttsModel=field('TTS model ID',settings.voice.tts_model_id||''),voice=field('Voice',settings.voice.voice||''),rate=field('Rate',settings.voice.rate,'number'),muted=check('Muted',settings.voice.muted);
  rate.input.min='0.5';rate.input.max='2';rate.input.step='0.1';
  form.append(stt.row,sttModel.row,tts.row,ttsModel.row,voice.row,rate.row,muted.row,el('p',{class:'muted'},`${models.items.length} configured model(s). Cloud audio requires matching approval.`));
  submit(form,'Save voice',()=>save('voice',{...settings.voice,stt:stt.node.value,stt_model_id:sttModel.input.value.trim()||null,tts:tts.node.value,tts_model_id:ttsModel.input.value.trim()||null,voice:voice.input.value.trim()||null,rate:Number(rate.input.value),muted:muted.node.checked}));
  card.append(form);return card;
}

function projectsCard(projects,tools){
  const card=formCard('Projects and coding tools');
  for(const project of projects.items)card.append(el('p',{class:'muted'},`${project.name} · ${project.root}`));
  for(const tool of tools.items)card.append(el('p',{class:'muted'},`${tool.kind} · ${tool.executable} · ${tool.capability.state}`));
  const projectForm=el('form'),name=field('Project name',''),root=field('Absolute project directory','');projectForm.append(name.row,root.row);
  submit(projectForm,'Register project',()=>request('POST','/projects',{name:name.input.value.trim(),root:root.input.value.trim(),allowed_paths:[],excluded_paths:[],notes:'',preferred_model_id:null,preferred_tool_id:null,owner_ack:true}));card.append(projectForm);
  const codingForm=el('form'),project=select('Coding project',projects.items[0]?.id||'',projects.items.map(item=>({value:item.id,label:item.name}))),goal=field('Coding goal','');codingForm.append(project.row,goal.row);
  submit(codingForm,'Create coding job',()=>request('POST','/jobs',{kind:'coding',project_id:project.node.value,task_id:null,conversation_id:null,goal:goal.input.value.trim(),mode:'plan_only',chain_id:null,model_override_id:null,tool_override_id:null,input_artifact_ids:[]}));card.append(codingForm);
  const toolForm=el('form'),id=field('Tool UUID',crypto.randomUUID()),executable=field('Absolute executable','/usr/bin/code'),kind=select('Tool kind','codex',['codex','compatible_cli','editor','chromium','pdf','clipboard','tts']);toolForm.append(id.row,kind.row,executable.row);
  submit(toolForm,'Save tool',()=>request('PUT',`/tools/${encodeURIComponent(id.input.value.trim())}`,{revision:0,kind:kind.node.value,executable:executable.input.value.trim(),enabled:true}));card.append(toolForm);return card;
}

function systemCard(settings,capabilities,resources){
  const card=formCard('Reminders, security and resources'),form=el('form'),timezone=field('Timezone',settings.assistant.timezone),start=field('Work starts',settings.assistant.work_start,'time'),end=field('Work ends',settings.assistant.work_end,'time'),checkins=field('Check-in minutes',settings.assistant.checkin_minutes??'','number'),notifications=check('Desktop notifications',settings.assistant.notifications),chatDays=field('Chat retention days',settings.security.chat_retention_days,'number'),eventDays=field('Event retention days',settings.security.event_retention_days,'number'),artifactDays=field('Artifact retention days',settings.security.artifact_retention_days,'number'),backupKeep=field('Backups to keep',settings.security.backup_keep,'number'),browserEnabled=check('Enable Chromium CDP',settings.browser.enabled),cdpUrl=field('Loopback CDP URL',settings.browser.cdp_url||'http://127.0.0.1:9222');
  form.append(timezone.row,start.row,end.row,checkins.row,notifications.row,chatDays.row,eventDays.row,artifactDays.row,backupKeep.row,browserEnabled.row,cdpUrl.row);
  submit(form,'Save assistant and security',async()=>{let current=await request('GET','/settings');current=await request('PATCH','/settings',{revision:current.revision,assistant:{...current.assistant,timezone:timezone.input.value.trim(),work_start:start.input.value,work_end:end.input.value,checkin_minutes:checkins.input.value?Number(checkins.input.value):null,notifications:notifications.node.checked}});await request('PATCH','/settings',{revision:current.revision,security:{...current.security,chat_retention_days:Number(chatDays.input.value),event_retention_days:Number(eventDays.input.value),artifact_retention_days:Number(artifactDays.input.value),backup_keep:Number(backupKeep.input.value)},browser:{...current.browser,enabled:browserEnabled.node.checked,cdp_mode:browserEnabled.node.checked?'attach':'disabled',cdp_url:browserEnabled.node.checked?cdpUrl.input.value.trim():null}})});
  card.append(form,el('pre',{class:'muted'},JSON.stringify({capabilities:capabilities.items,resources},null,2)));return card;
}

export async function mount(container){
  container.replaceChildren(el('h2',{},'Workspace'));const output=el('div');container.append(output);
  try{const [settings,providers,models,projects,tools,capabilities,resources]=await Promise.all([request('GET','/settings'),request('GET','/providers?limit=50'),request('GET','/models?limit=50'),request('GET','/projects?limit=50'),request('GET','/tools?limit=50'),request('GET','/capabilities'),request('GET','/system/resources')]);output.append(identityCard(settings),providersCard(providers,models),credentialCard(providers),voiceCard(settings,models),projectsCard(projects,tools),systemCard(settings,capabilities,resources))}catch(error){showError(output,error)}
  return()=>{};
}
