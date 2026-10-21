import {el,showError} from './dom.js';
import {request} from './api.js';
import {startCapture,stopCapture,stop as stopVoice} from './voice.js';

export function renderMessage(message){return el('article',{class:'card'},`${message.role}: ${message.text}`)}
export async function send(text,conversationId){return request('POST',`/conversations/${conversationId}/messages`,{text,model_override_id:null,confirmed_task:null})}

function renderReceipt(messages,conversation,receipt){
  messages.append(renderMessage(receipt.message));
  if(receipt.assistant_message)messages.append(renderMessage(receipt.assistant_message));
  if(!receipt.draft)return;
  const draft=el('article',{class:'card'});draft.append(el('pre',{},JSON.stringify(receipt.draft,null,2)));
  if(receipt.draft.task){
    const confirm=el('button',{type:'button'},'Confirm task');
    confirm.addEventListener('click',async()=>{confirm.disabled=true;try{const saved=await request('POST',`/conversations/${conversation.id}/messages`,{text:`Confirm: ${receipt.draft.task.title}`,model_override_id:null,confirmed_task:receipt.draft.task});draft.append(el('p',{class:'muted'},`Created task ${saved.task.title}`))}catch(error){showError(draft,error)}finally{confirm.disabled=false}});
    draft.append(confirm);
  }
  messages.append(draft);
}

export async function mount(container){
  container.replaceChildren(el('h2',{},'Chat'));
  const messages=el('div'),form=el('form'),input=el('textarea',{name:'message',placeholder:'Message Maya','aria-label':'Message Maya'}),submit=el('button',{type:'submit'},'Send'),voice=el('button',{type:'button','aria-label':'Hold to talk'},'Hold to talk');
  form.append(input,voice,submit);container.append(messages,form);
  let conversation,settings,capturing=false;
  const pendingJobs=new Map();
  const terminal=new Set(['completed','completed_with_issues','blocked','failed','cancelled','needs_reconciliation']);
  try{
    const [page,currentSettings]=await Promise.all([request('GET','/conversations?limit=50'),request('GET','/settings')]);
    settings=currentSettings.voice;
    conversation=page.items[0]||await request('POST','/conversations',{project_id:null,model_override_id:null,current_task_id:null});
    const history=await request('GET',`/conversations/${conversation.id}/messages?limit=100`);
    messages.replaceChildren(...history.items.map(renderMessage));
    voice.disabled=settings.stt==='disabled'||!settings.stt_model_id;
  }catch(error){showError(messages,error);return()=>{stopVoice()}}

  const beginVoice=async event=>{
    if(capturing)return;
    event.preventDefault();
    try{await startCapture(settings);capturing=true;voice.textContent='Release to transcribe';voice.setAttribute('aria-pressed','true');if(event.pointerId!==undefined)voice.setPointerCapture(event.pointerId)}catch(error){showError(messages,error)}
  };
  const endVoice=async event=>{
    if(!capturing)return;
    event.preventDefault();capturing=false;voice.disabled=true;
    try{const transcript=await stopCapture();input.value=transcript.text;input.focus()}catch(error){showError(messages,error)}finally{voice.disabled=false;voice.textContent='Hold to talk';voice.setAttribute('aria-pressed','false')}
  };
  voice.addEventListener('pointerdown',beginVoice);
  voice.addEventListener('pointerup',endVoice);
  voice.addEventListener('pointercancel',endVoice);
  voice.addEventListener('keydown',event=>{if((event.key===' '||event.key==='Enter')&&!event.repeat)beginVoice(event)});
  voice.addEventListener('keyup',event=>{if(event.key===' '||event.key==='Enter')endVoice(event)});

  const refreshJob=async jobId=>{
    if(!pendingJobs.has(jobId))return;
    const detail=await request('GET',`/jobs/${encodeURIComponent(jobId)}`);
    const state=detail.job.state,pending=pendingJobs.get(jobId);
    pending.textContent=`Assistant job: ${state.replaceAll('_',' ')}`;
    if(!terminal.has(state))return;
    if(state==='completed'||state==='completed_with_issues'){
      const history=await request('GET',`/conversations/${conversation.id}/messages?limit=100`);
      messages.replaceChildren(...history.items.map(renderMessage));
    }else{
      pending.className='error';
      pending.textContent=`Assistant response ${state.replaceAll('_',' ')}. Open Activity for recovery details.`;
    }
    pendingJobs.delete(jobId);
  };
  const onMayaEvent=event=>{
    const jobId=event.detail?.job_id;
    if(jobId&&pendingJobs.has(jobId))refreshJob(jobId).catch(error=>showError(messages,error));
  };
  addEventListener('maya:event',onMayaEvent);

  form.addEventListener('submit',async event=>{
    event.preventDefault();if(!input.value.trim())return;submit.disabled=true;
    try{const text=input.value,inputReceipt=await send(text,conversation.id);renderReceipt(messages,conversation,inputReceipt);input.value='';if(inputReceipt.job_id){const pending=el('p',{class:'muted'},'Assistant job: queued');messages.append(pending);pendingJobs.set(inputReceipt.job_id,pending);await refreshJob(inputReceipt.job_id)}}catch(error){showError(messages,error)}finally{submit.disabled=false}
  });
  return()=>{removeEventListener('maya:event',onMayaEvent);stopVoice()};
}
