
// P3-05 Light rigs nomeados + gizmo 3D + sun disk
export type LightRig = { id:string, labelKey:string, key:{azimuth:number,elevation:number,intensity:number,color:string}, fill?:any, rim?:any, back?:any, kicker?:any };
export const LIGHT_RIGS: LightRig[] = [
  { id:"studio_soft", labelKey:"rig.studio_soft", key:{azimuth:45,elevation:45,intensity:1.0,color:"#fff8e7"} },
  { id:"golden_hour", labelKey:"rig.golden_hour", key:{azimuth:-30,elevation:20,intensity:1.2,color:"#ffb86b"} },
  { id:"overcast", labelKey:"rig.overcast", key:{azimuth:0,elevation:80,intensity:0.85,color:"#d0e0ff"} },
  { id:"dramatic", labelKey:"rig.dramatic", key:{azimuth:70,elevation:15,intensity:1.4,color:"#ffd6a3"} },
];
// Gizmo 3D: draggable sun disk in viewport — emits set_light dir
