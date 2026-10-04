#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn execute(program: Program) -> HostValue {
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, Hosts::default()).unwrap();
    let result = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    result
}
async fn run(source: &str) -> HostValue {
    execute(compiler::compile(source, &Hosts::default()).unwrap()).await
}

#[tokio::test]
async fn option_is_null_or_payload_with_real_enum_identity_and_indices() {
    assert_eq!(run(r#"
    import {parse,stringify} from "json";
    alias Maybe=Option<number>;
    fn main() string {
        const empty:Maybe=None; const full:Maybe=Some(7);
        const a=match parse<Maybe>(stringify(empty)){Ok(None)=>"none",Ok(Some(x))=>"wrong",Err(e)=>e};
        const b=match "7".parseJSON<Maybe>(){Ok(Some(n))=>string(n),Ok(None)=>"wrong",Err(e)=>e};
        const typ=match JSON.parse<Maybe>("null"){Ok(n)=>n.getType().toString(),Err(e)=>e};
        return stringify(empty)+":"+full.toJSON()+":"+a+":"+b+":"+typ+":"+stringify(None);
    }"#).await,HostValue::String("null:7:none:7:Option:null".into()));
}

#[tokio::test]
async fn result_and_declared_enums_use_tags_and_extract_typed_payloads() {
    assert_eq!(run(r#"
    import {parse,stringify} from "json";
    alias Reply=Result<number,string>;
    enum State { Pending, Ready(number), Failed(string) }
    fn main() string {
        const ok:Reply=Ok(5);const err:Reply=Err("oops");
        const a=match parse<Reply>(stringify(ok)){Ok(Ok(x))=>string(x),Ok(Err(e))=>e,Err(e)=>e};
        const b=match parse<Reply>(stringify(err)){Ok(Err(e))=>e,Ok(Ok(x))=>"bad",Err(e)=>e};
        const c=match parse<State>(stringify(State.Ready(9))){Ok(Ready(n))=>string(n),Ok(Pending)=>"bad",Ok(Failed(e))=>e,Err(e)=>e};
        return stringify(ok)+";"+stringify(err)+";"+stringify(State.Pending)+";"+a+":"+b+":"+c;
    }"#).await,HostValue::String(r#"{"tag":"Ok","value":5};{"tag":"Err","value":"oops"};{"tag":"Pending"};5:oops:9"#.into()));
}

#[tokio::test]
async fn nested_struct_options_and_enum_payloads_keep_receiver_methods() {
    assert_eq!(
        run(r#"
    struct Item {value:number;}
    fn(i Item) twice() number{return i.value*2;}
    enum State {Missing,Found(Item)}
    struct Envelope {state:State;owner:Option<string>;}
    fn main() number {
        const item=Envelope{state:State.Found(Item{value:21}),owner:None};
        return match JSON.parse<Envelope>(item.toJSON()){
            Ok(e)=>match(e.state){Found(i)=>i.twice(),Missing=>0},
            Err(error)=>-1
        };
    }"#)
        .await,
        HostValue::Number(42.)
    );
}

#[tokio::test]
async fn newtypes_use_underlying_json_and_restore_their_brand_and_methods() {
    assert_eq!(run(r#"
    type Cents number
    type Label string
    type Active bool
    fn(c Cents) twice() Cents{return Cents(unboxNumber(c)*2);}
    fn main() string {
        const text=Cents(21).toJSON();
        const money=match JSON.parse<Cents>(text){Ok(c)=>string(unboxNumber(c.twice()))+":"+c.getType().toString(),Err(e)=>e};
        const label=match JSON.parse<Label>(Label("Deka").toJSON()){Ok(x)=>x.getType().toString()+":"+string(x),Err(e)=>e};
        const active=match JSON.parse<Active>(Active(true).toJSON()){Ok(x)=>x.getType().toString(),Err(e)=>e};
        return text+":"+money+":"+label+":"+active;
    }"#).await,HostValue::String("21:42:Cents:Label:Deka:Active".into()));
}

#[tokio::test]
async fn union_parsing_requires_exactly_one_member_including_nominal_ambiguity() {
    assert_eq!(
        run(r#"
    alias Choice=number|string;
    
    fn describe(value:Choice) string{return match(value){number(n)=>string(n),string(s)=>s};}
    fn main() string {
        const a=match JSON.parse<Choice>("7"){Ok(v)=>describe(v),Err(e)=>e};
        const b=match JSON.parse<Choice>("\"Deka\""){Ok(v)=>describe(v),Err(e)=>e};
        const no=match JSON.parse<Choice>("true"){Ok(v)=>"bad",Err(e)=>e};
        return a+":"+b+":"+no;
    }"#)
        .await,
        HostValue::String("7:Deka:$: JSON union matches no member".into())
    );
}

#[tokio::test]
async fn incorrect_tags_and_payloads_return_err_without_throwing() {
    assert_eq!(run(r#"
    enum State {Pending,Ready(number)}
    fn status(text:string) string {
        return match JSON.parse<State>(text){Ok(x)=>"bad",Err(e)=>"err"};
    }
    fn main() string {
        try {
            return status("{}")+":"+status("{\"tag\":\"Unknown\"}")+":"+status("{\"tag\":\"Ready\"}")+":"+status("{\"tag\":\"Ready\",\"value\":\"x\"}")+":"+status("{\"tag\":\"Pending\",\"value\":7}");
        }catch(e){return "threw";}
    }"#).await,HostValue::String("err:err:err:err:err".into()));
}

#[test]
fn directly_nested_options_are_rejected_through_all_checked_entry_points() {
    for source in [
        r#"alias Nested=Option<Option<number>>;const x=JSON.parse<Nested>("null");"#,
        r#"alias Nested=Option<Option<number>>;fn bad(x:Nested)string{return x.toJSON();}"#,
        r#"import {parse}from"json";alias Nested=Array<Option<Option<number>>>;const x=parse<Nested>("[]");"#,
        r#"struct Envelope {value:Option<Option<number>>;}const x=JSON.parse<Envelope>("{}");"#,
        r#"enum State {Nested(Option<Option<number>>)}const x=JSON.parse<State>("{}");"#,
    ] {
        let error = compiler::compile(source, &Hosts::default()).unwrap_err();
        assert!(
            error.contains("directly nested Option"),
            "{source}: {error}"
        );
    }
}

#[tokio::test]
async fn imported_enum_private_struct_factories_survive_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let model = dir.path().join("model.ds");
    let barrel = dir.path().join("barrel.ds");
    let main = dir.path().join("main.ds");
    std::fs::write(
        &model,
        r#"
        struct Item {value:number;}
        fn(i Item) twice() number{return i.value*2;}
        enum State {Missing,Found(Item)}
        export {State};
        export fn read(state:State) number {return match(state){Found(i)=>i.twice(),Missing=>0};}
        export const text=State.Found(Item{value:21}).toJSON();
    "#,
    )
    .unwrap();
    std::fs::write(&barrel, r#"export {State,read,text} from "./model.ds";"#).unwrap();
    std::fs::write(
        &main,
        r#"
        import {parse} from "json";
        import type {State} from "./barrel.ds";
        import {read,text} from "./barrel.ds";
        fn main() number {return match parse<State>(text){Ok(state)=>read(state),Err(e)=>-1};}
    "#,
    )
    .unwrap();
    let program = compiler::compile_file(&main, &Hosts::default(), Some("main")).unwrap();
    for path in [model, barrel, main] {
        std::fs::remove_file(path).unwrap();
    }
    assert_eq!(execute(program).await, HostValue::Number(42.));
}

#[tokio::test]
async fn same_wire_tag_from_distinct_modules_is_ambiguous_without_type_identity_guessing() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.ds");
    let second = dir.path().join("second.ds");
    let main = dir.path().join("main.ds");
    std::fs::write(&first, r#"enum First {Same(number)}export{First};"#).unwrap();
    std::fs::write(&second, r#"enum Second {Same(number)}export{Second};"#).unwrap();
    std::fs::write(&main,r#"
        import type {First}from"./first.ds";
        import type {Second}from"./second.ds";
        alias Ambiguous=First|Second;
        fn main() string {
            return match JSON.parse<Ambiguous>("{\"tag\":\"Same\",\"value\":7}"){Ok(v)=>"bad",Err(e)=>e};
        }
    "#).unwrap();
    let program = compiler::compile_file(&main, &Hosts::default(), Some("main")).unwrap();
    for path in [first, second, main] {
        std::fs::remove_file(path).unwrap();
    }
    assert_eq!(
        execute(program).await,
        HostValue::String("$: JSON union matches more than one member".into())
    );
}
