use helm_rs_events::{Event, EventLevel, EventSink, InMemoryEventSink};

#[test]
fn in_memory_sink_records_events_in_order() {
    let sink = InMemoryEventSink::default();

    sink.emit(Event::Log {
        level: EventLevel::Info,
        message: "starting".to_owned(),
    });
    sink.emit(Event::StepDetail {
        id: "apply".to_owned(),
        detail: "parsed 2 resources".to_owned(),
    });

    assert_eq!(
        sink.events(),
        vec![
            Event::Log {
                level: EventLevel::Info,
                message: "starting".to_owned(),
            },
            Event::StepDetail {
                id: "apply".to_owned(),
                detail: "parsed 2 resources".to_owned(),
            },
        ]
    );
}
