(() => {
  'use strict';

  const decoder = new TextDecoder();
  const bybitIntervals = {'1m':'1','3m':'3','5m':'5','15m':'15','30m':'30','1h':'60','2h':'120','4h':'240','1d':'D'};
  const mexcIntervals = {'1m':'Min1','5m':'Min5','15m':'Min15','30m':'Min30','1h':'Min60','4h':'Hour4','1d':'Day1'};
  const okxIntervals = {'1m':'1m','3m':'3m','5m':'5m','15m':'15m','30m':'30m','1h':'1H','2h':'2H','4h':'4H','1d':'1Dutc'};

  function milliseconds(value){const number=Number(value);return number>0&&number<1e12?number*1000:number}
  function candle(time,open,high,low,close,volume){
    const normalized={time:milliseconds(time),open:Number(open),high:Number(high),low:Number(low),close:Number(close)},parsedVolume=Number(volume);
    if(!Object.values(normalized).every(Number.isFinite)||normalized.time<0||normalized.open<=0||normalized.high<=0||normalized.low<=0||normalized.close<=0)return null;
    normalized.high=Math.max(normalized.high,normalized.open,normalized.close);normalized.low=Math.min(normalized.low,normalized.open,normalized.close);
    if(Number.isFinite(parsedVolume)&&parsedVolume>=0)normalized.volume=parsedVolume;
    return normalized;
  }
  function parseJson(data){if(typeof data!=='string')return null;try{return JSON.parse(data)}catch{return null}}
  function tickAggregator(intervalMillis){
    let current=null;
    return(time,price)=>{const timestamp=milliseconds(time),value=Number(price);if(!Number.isFinite(timestamp)||!Number.isFinite(value)||value<=0)return null;const bucket=Math.floor(timestamp/intervalMillis)*intervalMillis;if(!current||current.time!==bucket)current={time:bucket,open:value,high:value,low:value,close:value};else{current.high=Math.max(current.high,value);current.low=Math.min(current.low,value);current.close=value}return{...current}};
  }
  function binanceDescriptor(venue,market,interval,intervalMillis){
    if(venue==='binance_spot')return{url:`wss://stream.binance.com:9443/ws/${market.toLowerCase()}@kline_${interval}`,parse:data=>{const message=parseJson(data),row=message?.data?.k||message?.k,value=row&&candle(row.t,row.o,row.h,row.l,row.c,row.v);return value?[value]:[]}};
    const symbol=market.toLowerCase(),aggregate=tickAggregator(intervalMillis);let sawKline=false;
    return{url:'wss://fstream.binance.com/ws',subscribe:{method:'SUBSCRIBE',params:[`${symbol}@kline_${interval}`,`${symbol}@bookTicker`],id:1},kind:'BBO-midpoint fallback',parse:data=>{const message=parseJson(data),row=message?.data?.k||message?.k;if(row){sawKline=true;const value=candle(row.t,row.o,row.h,row.l,row.c,row.v);return value?[value]:[]}if(!sawKline&&(message?.e==='bookTicker'||message?.data?.e==='bookTicker')){const tick=message.data||message,bid=Number(tick.b),ask=Number(tick.a),value=aggregate(tick.E||tick.T||Date.now(),(bid+ask)/2);return value?[value]:[]}return[]}};
  }
  function bybitDescriptor(venue,market,interval){
    const topic=`kline.${bybitIntervals[interval]}.${market}`;
    return{url:`wss://stream.bybit.com/v5/public/${venue==='bybit_spot'?'spot':'linear'}`,subscribe:{op:'subscribe',args:[topic]},heartbeat:{op:'ping'},heartbeatMs:20_000,parse:data=>{const message=parseJson(data);if(message?.topic!==topic)return[];return(message.data||[]).map(row=>candle(row.start,row.open,row.high,row.low,row.close,row.volume)).filter(Boolean)}};
  }
  function hyperliquidDescriptor(market,interval){
    return{url:'wss://api.hyperliquid.xyz/ws',subscribe:{method:'subscribe',subscription:{type:'candle',coin:market,interval}},heartbeat:{method:'ping'},heartbeatMs:30_000,parse:data=>{const message=parseJson(data);if(message?.channel!=='candle')return[];const rows=Array.isArray(message.data)?message.data:[message.data];return rows.map(row=>row&&candle(row.t,row.o,row.h,row.l,row.c,row.v)).filter(Boolean)}};
  }
  async function lighterDescriptor(market,interval){
    const response=await fetch('https://mainnet.zklighter.elliot.ai/api/v1/orderBooks',{headers:{Accept:'application/json'}});if(!response.ok)throw new Error(`Lighter market lookup failed (${response.status})`);const body=await response.json(),rows=body.order_books||body.orderBooks||[],match=rows.find(row=>String(row.symbol).toUpperCase()===market.toUpperCase()),marketId=match?.market_id??match?.marketId;if(marketId==null)throw new Error(`Lighter live market id not found for ${market}`);const channel=`candle/${marketId}/${interval}`;
    return{url:'wss://mainnet.zklighter.elliot.ai/stream?readonly=true',subscribe:{type:'subscribe',channel},heartbeat:{type:'subscribe',channel},heartbeatMs:60_000,parse:data=>{const message=parseJson(data);if(!message?.candles||!String(message.channel||'').includes(`candle:${marketId}:`))return[];return message.candles.map(row=>candle(row.t,row.o,row.h,row.l,row.c,row.v)).filter(Boolean)}};
  }
  function asterDescriptor(market,interval){return{url:`wss://fstream.asterdex.com/ws/${market.toLowerCase()}@kline_${interval}`,parse:data=>{const message=parseJson(data),row=message?.data?.k||message?.k,value=row&&candle(row.t,row.o,row.h,row.l,row.c,row.v);return value?[value]:[]}}}
  function ondoDescriptor(market,intervalMillis){
    const aggregate=tickAggregator(intervalMillis),channel='markPricesPerps';
    return{url:'wss://api.ondoperps.xyz/ws',subscribe:{op:'subscribe',channel,markets:[market]},heartbeat:()=>({op:'ping',id:`basis-lab-${Date.now()}`}),heartbeatMs:10_000,kind:'mark-derived',parse:data=>{const message=parseJson(data);if(message?.type!=='update'||message.channel!==channel)return[];return(message.data||[]).filter(row=>row.market===market).map(row=>aggregate(Date.parse(row.lastUpdatedTime||message.timestamp),row.markPrice)).filter(Boolean)}};
  }
  function readVarint(bytes,start){let value=0,multiplier=1,index=start;while(index<bytes.length){const byte=bytes[index++];value+=(byte&127)*multiplier;if(!(byte&128))return[value,index];multiplier*=128;if(multiplier>Number.MAX_SAFE_INTEGER)throw new Error('protobuf varint overflow')}throw new Error('truncated protobuf varint')}
  function protobufFields(bytes){const fields=new Map();let index=0;while(index<bytes.length){const[tag,next]=readVarint(bytes,index);index=next;const field=Math.floor(tag/8),wire=tag%8;let value;if(wire===0)[value,index]=readVarint(bytes,index);else if(wire===2){let length;[length,index]=readVarint(bytes,index);value=bytes.slice(index,index+length);index+=length}else if(wire===1){value=bytes.slice(index,index+8);index+=8}else if(wire===5){value=bytes.slice(index,index+4);index+=4}else throw new Error(`unsupported protobuf wire type ${wire}`);if(!fields.has(field))fields.set(field,[]);fields.get(field).push(value)}return fields}
  function protoValue(fields,field){return fields.get(field)?.[0]}
  function protoText(fields,field){const value=protoValue(fields,field);return value instanceof Uint8Array?decoder.decode(value):''}
  async function mexcSpotCandles(data){
    let buffer;if(data instanceof ArrayBuffer)buffer=data;else if(typeof Blob!=='undefined'&&data instanceof Blob)buffer=await data.arrayBuffer();else return[];
    try{const wrapper=protobufFields(new Uint8Array(buffer)),payload=protoValue(wrapper,308);if(!(payload instanceof Uint8Array))return[];const row=protobufFields(payload),value=candle(protoValue(row,2),protoText(row,3),protoText(row,5),protoText(row,6),protoText(row,4),protoText(row,7));return value?[value]:[]}catch{return[]}
  }
  function mexcSpotDescriptor(market,interval){const channel=`spot@public.kline.v3.api.pb@${market}@${mexcIntervals[interval]}`;return{url:'wss://wbs-api.mexc.com/ws',subscribe:{method:'SUBSCRIPTION',params:[channel]},heartbeat:{method:'PING'},heartbeatMs:20_000,binaryType:'arraybuffer',parse:mexcSpotCandles}}
  function mexcPerpDescriptor(market,interval){const symbol=market.replaceAll('-','_');return{url:'wss://contract.mexc.com/edge',subscribe:{method:'sub.kline',param:{symbol,interval:mexcIntervals[interval]}},heartbeat:{method:'ping'},heartbeatMs:20_000,parse:data=>{const message=parseJson(data),row=message?.channel==='push.kline'?message.data:null,value=row&&candle(row.t,row.o,row.h,row.l,row.c,row.v??row.a);return value?[value]:[]}}}
  function okxDescriptor(market,interval){const channel=`candle${okxIntervals[interval]}`,arg={channel,instId:market};return{url:'wss://ws.okx.com:8443/ws/v5/business',subscribe:{op:'subscribe',args:[arg]},heartbeat:'ping',heartbeatMs:20_000,parse:data=>{const message=parseJson(data);if(message?.arg?.channel!==channel||message.arg.instId!==market)return[];return(message.data||[]).map(row=>candle(row[0],row[1],row[2],row[3],row[4],row[5])).filter(Boolean)}}}
  async function descriptor({venue,market,interval,intervalMillis}){
    if(!market)throw new Error('Choose a market before following live');
    switch(venue){
      case'binance_spot':case'binance_perp':return binanceDescriptor(venue,market,interval,intervalMillis);
      case'bybit_spot':case'bybit_perp':return bybitDescriptor(venue,market,interval);
      case'hyperliquid_perp':return hyperliquidDescriptor(market,interval);
      case'lighter_perp':return lighterDescriptor(market,interval);
      case'aster_perp':return asterDescriptor(market,interval);
      case'ondo_perp':return ondoDescriptor(market,intervalMillis);
      case'mexc_spot':return mexcSpotDescriptor(market,interval);
      case'mexc_perp':return mexcPerpDescriptor(market,interval);
      case'okx_spot':case'okx_perp':return okxDescriptor(market,interval);
      default:throw new Error(`Live WebSocket feed is not configured for ${venue}`);
    }
  }

  window.BasisLiveFeeds={descriptor};
})();
