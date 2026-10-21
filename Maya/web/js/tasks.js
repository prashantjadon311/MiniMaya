import {el,showError,formatTime} from './dom.js';
import {request} from './api.js';

export function dueAtMillis(value){
  if(!value)return null;
  const millis=new Date(value).getTime();
  if(!Number.isFinite(millis))throw new Error('Due date is invalid');
  return Math.trunc(millis);
}

export function isoWeek(date=new Date()){
  const day=new Date(Date.UTC(date.getUTCFullYear(),date.getUTCMonth(),date.getUTCDate()));
  day.setUTCDate(day.getUTCDate()+4-(day.getUTCDay()||7));
  const yearStart=new Date(Date.UTC(day.getUTCFullYear(),0,1));
  const week=Math.ceil((((day-yearStart)/86400000)+1)/7);
  return `${day.getUTCFullYear()}-W${String(week).padStart(2,'0')}`;
}

export function progressPatch(task,progress,status){
  return {revision:task.revision,status,progress_pct:progress,progress_source:'manual'};
}

export async function editTask(task){const{id,...patch}=task;return request('PATCH',`/tasks/${id}`,patch)}
export async function showReport(period='daily'){
  const timezone=Intl.DateTimeFormat().resolvedOptions().timeZone;
  const query=period==='weekly'?{week:isoWeek(),timezone}:{date:new Date().toISOString().slice(0,10),timezone};
  return request('GET',`/reports/${period}?${new URLSearchParams(query)}`);
}

function taskCard(task,reload){
  const card=el('article',{class:'card'}),title=el('strong',{},task.title),due=el('p',{class:'muted'},task.due_at?formatTime(task.due_at,task.timezone):'No due date'),controls=el('div',{class:'row'}),status=el('select');
  for(const value of ['todo','in_progress','blocked','done','cancelled']){const option=el('option',{value},value.replace('_',' '));option.selected=value===task.status;status.append(option)}
  const progress=el('input',{type:'number',value:task.progress_pct??0,'aria-label':'Progress percent'});progress.min='0';progress.max='100';
  const update=el('button',{type:'button'},'Update');
  update.addEventListener('click',async()=>{update.disabled=true;try{await editTask({id:task.id,...progressPatch(task,Number(progress.value),status.value)});await reload()}catch(error){showError(card,error)}finally{update.disabled=false}});
  controls.append(status,progress,update);card.append(title,due,controls);return card;
}

export async function mount(container){
  container.replaceChildren(el('h2',{},'Tasks'));
  const create=el('form'),title=el('input',{name:'title',placeholder:'New task title'}),due=el('input',{type:'datetime-local','aria-label':'Due date'}),add=el('button',{type:'submit'},'Add task'),reports=el('div',{class:'row'}),daily=el('button',{type:'button'},'Daily plan'),weekly=el('button',{type:'button'},'Weekly report'),report=el('pre',{class:'muted'}),list=el('div');
  create.append(title,due,add);reports.append(daily,weekly);container.append(create,reports,report,list);
  const reload=async()=>{try{const page=await request('GET','/tasks?limit=50');list.replaceChildren(...page.items.map(task=>taskCard(task,reload)))}catch(error){showError(list,error)}};
  const loadReport=async period=>{try{report.textContent=JSON.stringify(await showReport(period),null,2)}catch(error){showError(report,error)}};
  daily.addEventListener('click',()=>loadReport('daily'));weekly.addEventListener('click',()=>loadReport('weekly'));
  create.addEventListener('submit',async event=>{event.preventDefault();if(!title.value.trim())return;add.disabled=true;try{const timezone=Intl.DateTimeFormat().resolvedOptions().timeZone;await request('POST','/tasks',{title:title.value.trim(),description:'',project_id:null,parent_task_id:null,priority:'normal',due_at:dueAtMillis(due.value),timezone,estimated_minutes:null,reminder:null});title.value='';due.value='';await reload()}catch(error){showError(list,error)}finally{add.disabled=false}});
  await reload();return()=>{};
}
