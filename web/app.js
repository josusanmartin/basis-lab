(() => {
  'use strict';

  const $ = (selector) => document.querySelector(selector);
  const refs = {
    leftVenue: $('#left-venue'), rightVenue: $('#right-venue'), leftMarket: $('#left-market'), rightMarket: $('#right-market'),
    leftMarkets: $('#left-market-options'), rightMarkets: $('#right-market-options'), run: $('#run'), swap: $('#swap'), formula: $('#formula'),
    intervals: $('#intervals'), ranges: $('#ranges'), copyApi: $('#copy-api'), canvas: $('#chart'), wrap: $('#chart-wrap'),
    tooltip: $('#chart-tooltip'), loading: $('#chart-loading'), empty: $('#chart-empty'), healthDot: $('#health-dot'),
    healthLabel: $('#health-label'), chartPair: $('#chart-pair'), chartSubtitle: $('#chart-subtitle'), observations: $('#observations-body'),
    alignment: $('#alignment-note'), entryZ: $('#entry-z'), exitZ: $('#exit-z'), entryLabel: $('#entry-z-label'), exitLabel: $('#exit-z-label'),
    upper: $('#upper-entry'), lower: $('#lower-entry'), exitWindow: $('#exit-window'), resetBands: $('#reset-bands'), toast: $('#toast')
  };

  const fallbackVenues = [
    ['binance_spot','Binance Spot'],['binance_perp','Binance Perpetual'],['bybit_spot','Bybit Spot'],['bybit_perp','Bybit Perpetual'],
    ['hyperliquid_perp','Hyperliquid'],['lighter_perp','Lighter'],['aster_perp','Aster'],['ondo_perp','Ondo Perps'],
    ['mexc_spot','MEXC Spot'],['mexc_perp','MEXC Perpetual'],['okx_spot','OKX Spot'],['okx_perp','OKX Perpetual']
  ].map(([id,label]) => ({id,label,intervals:['1m','5m','15m','30m','1h','4h','1d']}));
  const defaultSymbols = {hyperliquid_perp:'BTC',lighter_perp:'BTC',ondo_perp:'BTC-USD.P',mexc_perp:'BTC_USDT',okx_spot:'BTC-USDT',okx_perp:'BTC-USDT-SWAP'};
  const allIntervals = ['1m','3m','5m','15m','30m','1h','2h','4h','1d'];
  const query = new URLSearchParams(location.search);
  const configuredApiBase = query.get('api_base');
  const hostedApiBase = 'https://basis-lab-sg.onrender.com';
  const hostedPreview = location.hostname.endsWith('.github.io') || location.hostname==='josusanmartin.com' || location.pathname.startsWith('/basis-lab/');
  const staticPreview = query.get('static_demo') === '1';
  const apiBase = (configuredApiBase || (hostedPreview ? hostedApiBase : location.origin)).replace(/\/$/,'');
  const state = {venues:fallbackVenues,interval:'1h',range:7,data:null,requestUrl:'',abort:null,visible:160,offset:0,hover:-1,drag:null,refreshTimer:null};
  const marketSearch = {left:{timer:null,abort:null,sequence:0,active:-1,items:[],open:false},right:{timer:null,abort:null,sequence:0,active:-1,items:[],open:false}};
  const metrics = {latest:$('#metric-latest'),mean:$('#metric-mean'),sigma:$('#metric-sigma'),z:$('#metric-z'),range:$('#metric-range'),direction:$('#metric-direction'),signal:$('#metric-signal')};

  function option(value,label){const element=document.createElement('option');element.value=value;element.textContent=label;return element}
  function populateVenues(){
    [refs.leftVenue,refs.rightVenue].forEach(select => {select.replaceChildren(...state.venues.map(v=>option(v.id,v.label)))});
    refs.leftVenue.value=queryValue('left_venue')||'bybit_perp'; refs.rightVenue.value=queryValue('right_venue')||'mexc_perp';
    refs.leftMarket.value=queryValue('left_market')||'WLFIUSDT'; refs.rightMarket.value=queryValue('right_market')||'WLFI_USDT';
    state.interval=queryValue('interval')||'1h'; state.range=Number(queryValue('range')||7);
    renderIntervals(); document.querySelectorAll('[data-range]').forEach(b=>b.classList.toggle('active',Number(b.dataset.range)===state.range));
  }
  function queryValue(name){return new URLSearchParams(location.search).get(name)}
  function venue(id){return state.venues.find(item=>item.id===id)||fallbackVenues.find(item=>item.id===id)}
  async function fetchApi(url,options={}){
    let response,lastError;
    for(let attempt=0;attempt<5;attempt++){
      try {response=await fetch(url,options);if(response.status!==404&&response.status<500)return response}
      catch(error){if(error.name==='AbortError')throw error;lastError=error}
      if(attempt<4)await new Promise(resolve=>setTimeout(resolve,250*(attempt+1)));
      if(options.signal?.aborted)throw new DOMException('Aborted','AbortError');
    }
    if(response)return response;throw lastError;
  }
  function renderIntervals(){
    const left=venue(refs.leftVenue.value)?.intervals||[],right=venue(refs.rightVenue.value)?.intervals||[];
    refs.intervals.replaceChildren(...allIntervals.map(name=>{const b=document.createElement('button');b.textContent=name.toUpperCase();b.dataset.interval=name;b.disabled=!left.includes(name)||!right.includes(name);b.classList.toggle('active',name===state.interval);return b}));
    if (refs.intervals.querySelector('.active:disabled')) {state.interval=[...refs.intervals.children].find(b=>!b.disabled)?.dataset.interval||'1h';renderIntervals()}
  }
  async function bootstrap(){
    try {if(staticPreview)throw new Error();const response=await fetchApi(`${apiBase}/api/v1/venues`,{headers:{Accept:'application/json'}});if(!response.ok)throw new Error();state.venues=(await response.json()).data;refs.healthDot.className='online';refs.healthLabel.textContent='Live'}
    catch {refs.healthDot.className=staticPreview?'preview':'error';refs.healthLabel.textContent=staticPreview?'Static demo':'API offline'}
    populateVenues(); bind(); loadMarkets('left');loadMarkets('right');await compare();
    state.refreshTimer=setInterval(()=>{if(!document.hidden)compare(true)},30000);
  }
  function bind(){
    refs.run.addEventListener('click',()=>compare());
    document.addEventListener('keydown',event=>{if(event.key==='Enter'&&document.activeElement?.tagName!=='BUTTON')compare()});
    bindMarketPicker('left');bindMarketPicker('right');
    document.addEventListener('pointerdown',event=>{if(!event.target.closest('.market-field')){closeMarketMenu('left');closeMarketMenu('right')}});
    [refs.leftVenue,refs.rightVenue].forEach((select,index)=>select.addEventListener('change',()=>{const side=index?'right':'left',market=index?refs.rightMarket:refs.leftMarket;market.value=defaultSymbols[select.value]||'BTCUSDT';closeMarketMenu(side);renderIntervals();loadMarkets(side)}));
    refs.swap.addEventListener('click',()=>{[refs.leftVenue.value,refs.rightVenue.value]=[refs.rightVenue.value,refs.leftVenue.value];[refs.leftMarket.value,refs.rightMarket.value]=[refs.rightMarket.value,refs.leftMarket.value];closeMarketMenu('left');closeMarketMenu('right');renderIntervals();loadMarkets('left');loadMarkets('right');compare()});
    refs.intervals.addEventListener('click',event=>{const button=event.target.closest('button:not(:disabled)');if(!button)return;state.interval=button.dataset.interval;renderIntervals();compare()});
    refs.ranges.addEventListener('click',event=>{const button=event.target.closest('button');if(!button)return;state.range=Number(button.dataset.range);refs.ranges.querySelectorAll('button').forEach(b=>b.classList.toggle('active',b===button));compare()});
    refs.copyApi.addEventListener('click',async()=>{try{await navigator.clipboard.writeText(state.requestUrl);toast('API URL copied')}catch{toast('Copy unavailable')}});
    [refs.entryZ,refs.exitZ].forEach(input=>input.addEventListener('input',()=>{updateBands();draw()}));
    refs.resetBands.addEventListener('click',()=>{refs.entryZ.value='2';refs.exitZ.value='.25';updateBands();draw()});
    addChartInteractions();
    new ResizeObserver(draw).observe(refs.wrap);
    window.addEventListener('beforeunload',()=>{state.abort?.abort();marketSearch.left.abort?.abort();marketSearch.right.abort?.abort();clearInterval(state.refreshTimer)},{once:true});
  }
  function marketPicker(side){return {select:side==='left'?refs.leftVenue:refs.rightVenue,input:side==='left'?refs.leftMarket:refs.rightMarket,list:side==='left'?refs.leftMarkets:refs.rightMarkets,search:marketSearch[side]}}
  function bindMarketPicker(side){
    const {input,list,search}=marketPicker(side);
    input.addEventListener('focus',()=>loadMarkets(side,true,''));
    input.addEventListener('input',()=>{clearTimeout(search.timer);search.timer=setTimeout(()=>loadMarkets(side,true,input.value.trim()),140)});
    input.addEventListener('keydown',event=>{
      if(event.key==='ArrowDown'||event.key==='ArrowUp'){
        event.preventDefault();
        if(!search.open){openMarketMenu(side);if(!search.items.length)loadMarkets(side,true,'');}
        const delta=event.key==='ArrowDown'?1:-1,index=search.active<0?(delta>0?0:search.items.length-1):(search.active+delta+search.items.length)%Math.max(1,search.items.length);setActiveMarket(side,index);return;
      }
      if(event.key==='Enter'&&search.open){event.preventDefault();event.stopPropagation();if(search.items.length)selectMarket(side,search.active<0?0:search.active);return}
      if(event.key==='Escape'&&search.open){event.preventDefault();closeMarketMenu(side)}
    });
    list.addEventListener('pointerdown',event=>event.preventDefault());
    list.addEventListener('click',event=>{const row=event.target.closest('.market-option');if(row)selectMarket(side,Number(row.dataset.index))});
  }
  function openMarketMenu(side){const {input,list,search}=marketPicker(side);search.open=true;list.hidden=false;input.setAttribute('aria-expanded','true')}
  function closeMarketMenu(side){const {input,list,search}=marketPicker(side);search.open=false;search.active=-1;list.hidden=true;input.setAttribute('aria-expanded','false');input.removeAttribute('aria-activedescendant')}
  function setActiveMarket(side,index){
    const {input,list,search}=marketPicker(side),rows=[...list.querySelectorAll('.market-option')];
    if(!rows.length){search.active=-1;return}
    search.active=Math.max(0,Math.min(index,rows.length-1));
    rows.forEach((row,rowIndex)=>{const active=rowIndex===search.active;row.classList.toggle('active',active);row.setAttribute('aria-selected',String(active))});
    input.setAttribute('aria-activedescendant',rows[search.active].id);rows[search.active].scrollIntoView({block:'nearest'});
  }
  function selectMarket(side,index){const {input,search}=marketPicker(side),market=search.items[index];if(!market)return;input.value=market.symbol;closeMarketMenu(side);input.focus()}
  function marketMessage(list,message){const row=document.createElement('div');row.className='market-option-state';row.setAttribute('role','option');row.setAttribute('aria-disabled','true');row.textContent=message;list.replaceChildren(row)}
  function renderMarketOptions(side,items){
    const {list,search}=marketPicker(side);search.items=items;search.active=-1;
    if(!items.length){marketMessage(list,'No matching markets');return}
    list.replaceChildren(...items.map((market,index)=>{const row=document.createElement('button'),primary=document.createElement('strong'),native=document.createElement('span');row.type='button';row.id=`${side}-market-option-${index}`;row.className='market-option';row.dataset.index=String(index);row.setAttribute('role','option');row.setAttribute('aria-selected','false');primary.textContent=market.normalized_symbol||market.symbol;native.textContent=market.symbol===market.normalized_symbol?'Native symbol':market.symbol;row.append(primary,native);return row}));
  }
  async function loadMarkets(side,open=false,term){
    const {select,input,list,search}=marketPicker(side);if(open)openMarketMenu(side);
    if(staticPreview){renderMarketOptions(side,[{symbol:input.value,normalized_symbol:input.value}]);return}
    search.abort?.abort();search.abort=new AbortController();const sequence=++search.sequence,query=term??input.value.trim();
    if(open)marketMessage(list,'Searching cached tickers…');
    try {const q=new URLSearchParams({venue:select.value,query,limit:'100'}),response=await fetchApi(`${apiBase}/api/v1/markets?${q}`,{signal:search.abort.signal,headers:{Accept:'application/json'}});if(!response.ok)throw new Error();const data=(await response.json()).data;if(sequence!==search.sequence)return;renderMarketOptions(side,data);if(search.open)openMarketMenu(side)}
    catch(error){if(error.name==='AbortError')return;if(sequence===search.sequence){marketMessage(list,'Market search unavailable');if(search.open)openMarketMenu(side)}}
  }
  function limits(){
    const end=Date.now(),start=end-state.range*864e5;
    const intervalMs={ '1m':6e4,'3m':18e4,'5m':3e5,'15m':9e5,'30m':18e5,'1h':36e5,'2h':72e5,'4h':144e5,'1d':864e5}[state.interval];
    return {from:start,to:end,limit:Math.min(1500,Math.ceil((end-start)/intervalMs)+4)};
  }
  function makeUrl(){const window=limits();return `${apiBase}/api/v1/compare?${new URLSearchParams({left_venue:refs.leftVenue.value,left_market:refs.leftMarket.value.trim().toUpperCase(),right_venue:refs.rightVenue.value,right_market:refs.rightMarket.value.trim().toUpperCase(),interval:state.interval,from:String(window.from),to:String(window.to),limit:String(window.limit),scale:'10000'})}`}
  async function compare(silent=false){
    if(!refs.leftMarket.value.trim()||!refs.rightMarket.value.trim())return toast('Choose both markets');
    state.abort?.abort();state.abort=new AbortController();state.requestUrl=makeUrl();
    refs.run.disabled=true;if(!silent){refs.loading.hidden=false;refs.empty.hidden=true}
    try {
      if(staticPreview){state.data=previewData();state.visible=Math.min(state.data.candles.length,Math.max(80,Math.round(refs.wrap.clientWidth/7)));state.offset=0;state.hover=-1;updateUrl();renderData();refs.healthDot.className='preview';refs.healthLabel.textContent='Static demo';return}
      const response=await fetchApi(state.requestUrl,{signal:state.abort.signal,headers:{Accept:'application/json'}});const body=await response.json().catch(()=>({}));
      if(!response.ok)throw new Error(body.error?.message||`Request failed (${response.status})`);
      state.data=body;state.visible=Math.min(body.candles.length,Math.max(80,Math.round(refs.wrap.clientWidth/7)));state.offset=0;state.hover=-1;
      updateUrl();renderData();refs.healthDot.className='online';refs.healthLabel.textContent='Live';
    } catch(error) {if(error.name==='AbortError')return;refs.empty.hidden=false;refs.empty.querySelector('strong').textContent='Comparison unavailable';refs.empty.querySelector('p').textContent=error.message;refs.healthDot.className='error';refs.healthLabel.textContent='Data error';toast(error.message)}
    finally {refs.loading.hidden=true;refs.run.disabled=false}
  }
  function updateUrl(){const params=new URLSearchParams({left_venue:refs.leftVenue.value,left_market:refs.leftMarket.value.toUpperCase(),right_venue:refs.rightVenue.value,right_market:refs.rightMarket.value.toUpperCase(),interval:state.interval,range:String(state.range)});if(configuredApiBase)params.set('api_base',configuredApiBase);if(staticPreview)params.set('static_demo','1');history.replaceState(null,'',`${location.pathname}?${params}`)}
  function previewData(){
    const {from,to,limit}=limits(),step={'1m':6e4,'3m':18e4,'5m':3e5,'15m':9e5,'30m':18e5,'1h':36e5,'2h':72e5,'4h':144e5,'1d':864e5}[state.interval],count=Math.min(limit,220),seed=[...`${refs.leftVenue.value}:${refs.leftMarket.value}:${refs.rightVenue.value}:${refs.rightMarket.value}`].reduce((sum,char)=>sum+char.charCodeAt(0),0),end=Math.floor(to/step)*step;
    const candles=Array.from({length:count},(_,index)=>{const time=Math.max(from,end-(count-1-index)*step),center=Math.sin((index+seed)/11)*18+Math.cos((index+seed)/29)*7,open=center+Math.sin(index*.73)*3,close=center+Math.cos(index*.51)*3.2,leftClose=1+seed/10000+index/100000;return {time,open,high:Math.max(open,close)+4.4,low:Math.min(open,close)-4.4,close,left_close:leftClose,right_close:leftClose/(1+close/10000)}}),closes=candles.map(c=>c.close),mean=closes.reduce((a,b)=>a+b,0)/closes.length,variance=closes.reduce((sum,value)=>sum+(value-mean)**2,0)/closes.length,standard_deviation=Math.sqrt(variance),latest=closes.at(-1);
    return {interval:state.interval,unit:'basis points · illustrative static data',matched_candles:count,dropped_left:0,dropped_right:0,candles,stats:{latest,mean,standard_deviation,minimum:Math.min(...closes),maximum:Math.max(...closes),z_score:(latest-mean)/standard_deviation}}
  }
  function renderData(){
    const data=state.data,stats=data.stats,left=refs.leftMarket.value.toUpperCase(),right=refs.rightMarket.value.toUpperCase();
    refs.formula.textContent=`(${venue(refs.leftVenue.value).label}:${left} ÷ ${venue(refs.rightVenue.value).label}:${right} − 1) × 10,000`;
    refs.chartPair.textContent=`${left} / ${right}`;refs.chartSubtitle.textContent=`${data.candles.length.toLocaleString()} aligned ${data.interval} candles · ${data.unit}`;
    setMetric(metrics.latest,formatBps(stats.latest),stats.latest);setMetric(metrics.mean,formatBps(stats.mean),stats.mean);metrics.sigma.textContent=`${Math.abs(stats.standard_deviation).toFixed(1)} bp`;setMetric(metrics.z,`${stats.z_score>=0?'+':''}${stats.z_score.toFixed(2)}σ`,stats.z_score);
    metrics.range.textContent=`${stats.minimum.toFixed(0)} → ${stats.maximum.toFixed(0)}`;metrics.direction.textContent=stats.latest>=0?'A trades at a premium':'A trades at a discount';metrics.signal.textContent=signal(stats.latest).label;
    refs.alignment.textContent=`${data.matched_candles} matched · ${data.dropped_left} A-only · ${data.dropped_right} B-only`;
    updateBands();renderTable();refs.empty.hidden=!!data.candles.length;draw();
  }
  function setMetric(element,text,value){element.textContent=text;element.classList.toggle('positive',value>0);element.classList.toggle('negative',value<0)}
  function formatBps(value){return `${value>=0?'+':''}${value.toFixed(Math.abs(value)>=100?0:1)} bp`}
  function thresholds(){if(!state.data)return null;const {mean,standard_deviation:s}=state.data.stats,entry=Number(refs.entryZ.value),exit=Number(refs.exitZ.value);return {upper:mean+entry*s,lower:mean-entry*s,exitUpper:mean+exit*s,exitLower:mean-exit*s,mean}}
  function updateBands(){refs.entryLabel.textContent=`${Number(refs.entryZ.value).toFixed(2).replace(/0$/,'')}σ`;refs.exitLabel.textContent=`${Number(refs.exitZ.value).toFixed(2).replace(/0$/,'')}σ`;const t=thresholds();if(!t)return;refs.upper.textContent=formatBps(t.upper);refs.lower.textContent=formatBps(t.lower);refs.exitWindow.textContent=`${t.exitLower.toFixed(1)} to ${t.exitUpper.toFixed(1)} bp`}
  function signal(value){const t=thresholds();if(!t)return {label:'NEUTRAL',className:''};if(value>=t.upper)return {label:'SHORT A / LONG B',className:'short'};if(value<=t.lower)return {label:'LONG A / SHORT B',className:'long'};if(value>=t.exitLower&&value<=t.exitUpper)return {label:'EXIT WINDOW',className:''};return {label:'WATCH',className:''}}
  function renderTable(){const rows=state.data.candles.slice(-12).reverse();refs.observations.replaceChildren(...rows.map(c=>{const tr=document.createElement('tr'),sig=signal(c.close);[new Date(c.time).toISOString().replace('T',' ').slice(0,16),formatBps(c.close),`${c.low.toFixed(1)} — ${c.high.toFixed(1)}`,price(c.left_close),price(c.right_close)].forEach(value=>{const td=document.createElement('td');td.textContent=value;tr.append(td)});const td=document.createElement('td'),pill=document.createElement('span');pill.className=`signal-pill ${sig.className}`;pill.textContent=sig.label;td.append(pill);tr.append(td);return tr}))}
  function price(value){return value>=1000?value.toLocaleString(undefined,{maximumFractionDigits:2}):value.toLocaleString(undefined,{maximumSignificantDigits:8})}
  function visibleCandles(){if(!state.data)return[];const all=state.data.candles,count=Math.min(state.visible,all.length),end=Math.max(count,all.length-state.offset),start=Math.max(0,end-count);return all.slice(start,end)}
  function draw(){
    const canvas=refs.canvas,rect=refs.wrap.getBoundingClientRect(),dpr=Math.min(devicePixelRatio||1,2);if(rect.width<10||rect.height<10)return;canvas.width=Math.round(rect.width*dpr);canvas.height=Math.round(rect.height*dpr);const ctx=canvas.getContext('2d');ctx.setTransform(dpr,0,0,dpr,0,0);ctx.clearRect(0,0,rect.width,rect.height);
    const candles=visibleCandles();if(!candles.length)return;const pad={top:18,right:63,bottom:31,left:8},w=rect.width-pad.left-pad.right,h=rect.height-pad.top-pad.bottom,t=thresholds();let min=Math.min(...candles.map(c=>c.low),t?.lower??Infinity,t?.exitLower??Infinity,0),max=Math.max(...candles.map(c=>c.high),t?.upper??-Infinity,t?.exitUpper??-Infinity,0);const margin=Math.max((max-min)*.08,1);min-=margin;max+=margin;const y=v=>pad.top+(max-v)/(max-min)*h,x=i=>pad.left+(i+.5)/candles.length*w;
    ctx.font='9px "DM Mono",monospace';ctx.textAlign='left';ctx.textBaseline='middle';for(let i=0;i<=5;i++){const yy=pad.top+h*i/5,value=max-(max-min)*i/5;ctx.strokeStyle='#1c222b';ctx.lineWidth=1;ctx.beginPath();ctx.moveTo(pad.left,yy+.5);ctx.lineTo(pad.left+w,yy+.5);ctx.stroke();ctx.fillStyle='#697483';ctx.fillText(`${value.toFixed(Math.abs(value)>100?0:1)}`,pad.left+w+8,yy)}
    const band=(value,color,dash=[])=>{const yy=y(value);ctx.save();ctx.strokeStyle=color;ctx.setLineDash(dash);ctx.beginPath();ctx.moveTo(pad.left,yy);ctx.lineTo(pad.left+w,yy);ctx.stroke();ctx.restore()};
    if(t){ctx.fillStyle='rgba(200,255,100,.025)';ctx.fillRect(pad.left,y(t.exitUpper),w,y(t.exitLower)-y(t.exitUpper));band(t.upper,'rgba(255,107,122,.42)',[4,5]);band(t.lower,'rgba(98,239,189,.42)',[4,5]);band(t.mean,'rgba(150,160,175,.6)',[2,4])}band(0,'rgba(200,255,100,.22)');
    const step=w/candles.length,body=Math.max(1,Math.min(step*.66,9));candles.forEach((c,i)=>{const xx=x(i),up=c.close>=c.open,color=up?'#62efbd':'#ff6b7a';ctx.strokeStyle=color;ctx.fillStyle=color;ctx.lineWidth=1;ctx.beginPath();ctx.moveTo(Math.round(xx)+.5,y(c.high));ctx.lineTo(Math.round(xx)+.5,y(c.low));ctx.stroke();const top=y(Math.max(c.open,c.close)),bottom=y(Math.min(c.open,c.close));ctx.fillRect(xx-body/2,top,body,Math.max(1,bottom-top))});
    const ticks=Math.min(rect.width<500?3:6,candles.length);for(let i=0;i<ticks;i++){const index=Math.round(i*(candles.length-1)/Math.max(1,ticks-1)),xx=x(index);ctx.fillStyle='#657080';ctx.textAlign=i===0?'left':i===ticks-1?'right':'center';ctx.fillText(timeLabel(candles[index].time,state.range),xx,pad.top+h+18)}
    if(state.hover>=0&&state.hover<candles.length){const c=candles[state.hover],xx=x(state.hover);ctx.strokeStyle='rgba(180,190,203,.35)';ctx.setLineDash([3,3]);ctx.beginPath();ctx.moveTo(xx,pad.top);ctx.lineTo(xx,pad.top+h);ctx.stroke();ctx.setLineDash([]);showTooltip(c,xx,y(c.close),rect)}else refs.tooltip.hidden=true;
  }
  function timeLabel(timestamp,range){const d=new Date(timestamp);return range<=1?d.toISOString().slice(11,16):range<=7?`${d.toISOString().slice(5,10)} ${d.toISOString().slice(11,16)}`:d.toISOString().slice(5,10)}
  function showTooltip(c,x,y,rect){refs.tooltip.hidden=false;refs.tooltip.replaceChildren();const strong=document.createElement('strong');strong.textContent=`${new Date(c.time).toISOString().slice(0,16).replace('T',' ')} UTC`;refs.tooltip.append(strong);[['O',c.open],['H',c.high],['L',c.low],['C',c.close]].forEach(([label,value])=>{const row=document.createElement('div'),a=document.createElement('span'),b=document.createElement('span');a.textContent=label;b.textContent=formatBps(value);row.append(a,b);refs.tooltip.append(row)});refs.tooltip.style.left=`${Math.min(rect.width-190,Math.max(8,x+13))}px`;refs.tooltip.style.top=`${Math.min(rect.height-115,Math.max(8,y-56))}px`}
  function addChartInteractions(){
    refs.canvas.addEventListener('pointermove',event=>{const rect=refs.canvas.getBoundingClientRect(),candles=visibleCandles(),right=63,left=8,index=Math.floor((event.clientX-rect.left-left)/(rect.width-left-right)*candles.length);state.hover=Math.max(-1,Math.min(candles.length-1,index));if(state.drag){const delta=Math.round((event.clientX-state.drag.x)/(rect.width-left-right)*state.visible);state.offset=Math.max(0,Math.min(state.data.candles.length-state.visible,state.drag.offset+delta))}draw()});
    refs.canvas.addEventListener('pointerleave',()=>{state.hover=-1;state.drag=null;draw()});
    refs.canvas.addEventListener('pointerdown',event=>{if(!state.data)return;refs.canvas.setPointerCapture(event.pointerId);state.drag={x:event.clientX,offset:state.offset}});
    refs.canvas.addEventListener('pointerup',()=>{state.drag=null});
    refs.canvas.addEventListener('wheel',event=>{if(!state.data)return;event.preventDefault();const factor=event.deltaY>0?1.14:.86;state.visible=Math.max(20,Math.min(state.data.candles.length,Math.round(state.visible*factor)));state.offset=Math.min(state.offset,Math.max(0,state.data.candles.length-state.visible));draw()},{passive:false});
    refs.canvas.addEventListener('dblclick',()=>{if(!state.data)return;state.visible=Math.min(state.data.candles.length,Math.max(80,Math.round(refs.wrap.clientWidth/7)));state.offset=0;draw()});
  }
  function toast(message){refs.toast.textContent=message;refs.toast.classList.add('show');clearTimeout(toast.timer);toast.timer=setTimeout(()=>refs.toast.classList.remove('show'),2200)}
  bootstrap();
})();
