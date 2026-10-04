#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
async fn run(source: &str) -> HostValue {
    let program = compiler::compile(source, &Hosts::default()).unwrap();
    execute(program).await
}
async fn execute(program: Program) -> HostValue {
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, Hosts::default()).unwrap();
    let result = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    result
}
#[tokio::test]
async fn imported_aliases_convert_primitives_and_lists_with_concrete_results() {
    assert_eq!(run(r#"import {parse as decode, stringify as encode} from "json";
    alias Numbers = Array<number>;
    fn main() string {
        const values=match decode<Numbers>(encode([1,2,-3.5])){Ok(xs)=>xs,Err(error)=>[]};
        const text=match decode<string>(encode("snow:雪\n")){Ok(s)=>s,Err(error)=>error};
        const n=values.has(2)?values[2]:0;return string(values.length)+":"+string(n)+":"+text+":"+encode(true);
    }"#).await,HostValue::String("3:-3.5:snow:雪\n:true".into()));
}
#[tokio::test]
async fn imported_struct_conversion_keeps_nominal_methods_and_required_field_format() {
    assert_eq!(run(r#"import {parse,stringify} from "json";
    struct Person {name:string;}
    interface Greeter {fn greet() string;}
    fn(p Person) greet() string {return "Hello, "+p.name;}
    fn welcome(g:Greeter) string {return g.greet();}
    fn main() string {
        const text=stringify(Person{name:"Deka"});
        return match parse<Person>(text){Ok(p)=>text+";"+welcome(p)+";"+p.getType().toString(),Err(error)=>error};
    }"#).await,HostValue::String("{\"Person\":{\"name\":\"Deka\"}};Hello, Deka;Person".into()));
}
#[tokio::test]
async fn imported_parse_errors_are_results_and_argument_runs_once() {
    assert_eq!(
        run(r#"import {parse,stringify} from "json";
    fn main() string {
        let calls=0;const input=fn() string {calls=calls+1;return "7";};
        const n=match parse<number>(input()){Ok(x)=>x,Err(error)=>0};
        try {
            const invalid=match parse<number>("{"){Ok(x)=>"bad",Err(error)=>"syntax"};
            const wrong=match parse<number>("\"wrong\""){Ok(x)=>"bad",Err(error)=>"shape"};
            return string(n)+":"+string(calls)+":"+invalid+":"+wrong;
        }catch(error){return "threw";}
    }"#)
        .await,
        HostValue::String("7:1:syntax:shape".into())
    );
}
#[test]
fn imported_functions_require_checked_shapes_and_cannot_be_function_values() {
    for (source, message) in [
        (
            r#"import type {parse} from "json";fn main(){parse<number>("7");}"#,
            "can only be used as a type",
        ),
        (
            r#"import {parse} from "json";fn main(){parse<number>(7);}"#,
            "expects a string",
        ),
        (
            r#"import {parse} from "json";fn main(){parse("7");}"#,
            "exactly one type argument",
        ),
        (
            r#"import {stringify} from "json";fn main(){return stringify(Some(7));}"#,
            "type-mapping decision",
        ),
        (
            r#"import {parse} from "json";fn main(){const decoder=parse;return decoder;}"#,
            "function values are not supported",
        ),
        (
            r#"import {stringify} from "json";fn main(){return stringify<number>(7);}"#,
            "does not accept type arguments",
        ),
    ] {
        let error = compiler::compile(source, &Hosts::default()).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
}
#[tokio::test]
async fn json_reexport_barrel_and_struct_factory_survive_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let barrel = dir.path().join("codec.ds");
    let main = dir.path().join("main.ds");
    std::fs::write(
        &barrel,
        r#"export {parse as decode,stringify as encode} from "json";"#,
    )
    .unwrap();
    std::fs::write(&main,r#"import {decode,encode} from "./codec.ds";
    struct Item {value:number;}
    fn(item Item) twice() number{return item.value*2;}
    fn main() number{return match decode<Item>(encode(Item{value:21})){Ok(item)=>item.twice(),Err(error)=>0};}"#).unwrap();
    let program = compiler::compile_file(&main, &Hosts::default(), Some("main")).unwrap();
    std::fs::remove_file(main).unwrap();
    std::fs::remove_file(barrel).unwrap();
    assert_eq!(execute(program).await, HostValue::Number(42.));
}
