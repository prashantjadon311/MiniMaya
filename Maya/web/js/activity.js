import {el,showError,confirmApproval} from './dom.js';
import {request} from './api.js';

export const showJob=id=>request('GET',`/jobs/${id}`);
export async function decide(approval,decision){return request('POST',`/approvals/${approval.id}/decision`,{revision:approval.revision,request_digest:approval.request_digest,decision,owner_ack:true})}

function actionButton(card,label,handler){
  const button=el('button',{type:'button'},label);
  button.addEventListener('click',async()=>{button.disabled=true;try{await handler()}catch(error){showError(card,error)}finally{button.disabled=false}});
  return button;
}

export async function mount(container){
  container.replaceChildren(el('h2',{},'Activity'));const list=el('div');container.append(list);
  const reload=async()=>{
    try{
      const page=await request('GET','/jobs?limit=50');list.replaceChildren();
      for(const job of page.items){
        const card=el('article',{class:'card'});card.append(el('strong',{},job.goal),el('p',{class:'muted'},`${job.state} · revision ${job.revision}`));
        const details=actionButton(card,'Details',async()=>{
          const detail=await showJob(job.id);card.querySelector('pre')?.remove();card.append(el('pre',{},JSON.stringify(detail,null,2)));
          if(detail.job.state==='planned'&&detail.plans.length){
            const plan=detail.plans.at(-1);
            card.append(actionButton(card,'Request implementation approval',async()=>{
              await request('POST',`/jobs/${job.id}/approval-requests`,{job_revision:detail.job.revision,action:'project_edit',plan_id:plan.id,change_digest:plan.digest,ttl_minutes:15});await reload();
            }));
          }
          for(const approval of detail.approvals.filter(item=>item.decision==='pending')){
            card.append(actionButton(card,`Review ${approval.action}`,async()=>{const choice=await confirmApproval(approval);await decide(approval,choice);await reload()}));
          }
          for(const action of detail.allowed_actions){
            let recovery;
            if(action==='reconcile'){
              recovery=el('select',{'aria-label':'Reconciliation action'});
              for(const value of ['inspect','continue_verification','replan','restore'])recovery.append(el('option',{value},value.replaceAll('_',' ')));
              card.append(recovery);
            }
            card.append(actionButton(card,action.replaceAll('_',' '),async()=>{await request('POST',`/jobs/${job.id}/actions`,{revision:detail.job.revision,action,step_id:null,reconciliation:recovery?.value??null,checkpoint_id:null});await reload()}));
          }
        });
        card.append(details);list.append(card);
      }
    }catch(error){showError(list,error)}
  };
  await reload();return()=>{};
}
