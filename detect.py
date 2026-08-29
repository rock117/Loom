#!/usr/bin/env python3.11
"""
YOLO Object Detection with RKNN
"""

import os
import sys
import cv2
import numpy as np
from rknnlite.api import RKNNLite

# COCO labels
LABELS_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'coco_labels.txt')

def load_labels(path):
    """Load COCO labels from file."""
    with open(path, 'r') as f:
        labels = [line.strip() for line in f.readlines()]
    return labels

def sigmoid(x):
    """Sigmoid activation function."""
    return 1 / (1 + np.exp(-np.clip(x, -50, 50)))

def yolov5_post_process(output_data, input_shape, conf_threshold=0.45, iou_threshold=0.45):
    """
    YOLOv5 post-processing.
    
    Args:
        output_data: Model output (list of arrays for different scales)
        input_shape: Model input shape [height, width]
        conf_threshold: Confidence threshold
        iou_threshold: NMS IoU threshold
    
    Returns:
        boxes: Bounding boxes [[x1, y1, x2, y2], ...]
        classes: Class indices
        scores: Confidence scores
    """
    boxes = []
    scores = []
    classes = []
    
    # YOLOv5 outputs 3 scales
    for i, output in enumerate(output_data):
        # output shape: [1, 3*(5+80), H, W] -> reshape to [1, 3, H, W, 85]
        batch_size, channels, height, width = output.shape
        num_anchors = 3
        num_classes = 80
        
        # Reshape
        output = output.reshape(batch_size, num_anchors, 5 + num_classes, height, width)
        output = output.transpose(0, 1, 3, 4, 2)  # [1, 3, H, W, 85]
        
        # Get anchors for this scale
        if i == 0:  # Small objects
            anchors = [[10, 13], [16, 30], [33, 23]]
        elif i == 1:  # Medium objects
            anchors = [[30, 61], [62, 45], [59, 119]]
        else:  # Large objects
            anchors = [[116, 90], [156, 198], [373, 326]]
        
        for a in range(num_anchors):
            for cy in range(height):
                for cx in range(width):
                    # Get raw predictions
                    raw = output[0, a, cy, cx, :]
                    
                    # Objectness score
                    obj_score = sigmoid(raw[4])
                    if obj_score < conf_threshold:
                        continue
                    
                    # Class scores
                    class_scores = sigmoid(raw[5:])
                    class_idx = np.argmax(class_scores)
                    class_score = class_scores[class_idx]
                    
                    # Final confidence
                    confidence = obj_score * class_score
                    if confidence < conf_threshold:
                        continue
                    
                    # Decode box
                    tx, ty, tw, th = raw[0], raw[1], raw[2], raw[3]
                    
                    # Grid offset
                    cx_offset = sigmoid(tx)
                    cy_offset = sigmoid(ty)
                    
                    # Box center
                    box_cx = (cx + cx_offset) * (input_shape[1] / width)
                    box_cy = (cy + cy_offset) * (input_shape[0] / height)
                    
                    # Box size
                    anchor_w, anchor_h = anchors[a]
                    box_w = np.exp(tw) * anchor_w
                    box_h = np.exp(th) * anchor_h
                    
                    # Convert to x1, y1, x2, y2
                    x1 = box_cx - box_w / 2
                    y1 = box_cy - box_h / 2
                    x2 = box_cx + box_w / 2
                    y2 = box_cy + box_h / 2
                    
                    boxes.append([x1, y1, x2, y2])
                    scores.append(float(confidence))
                    classes.append(class_idx)
    
    # Apply NMS
    if len(boxes) > 0:
        indices = cv2.dnn.NMSBoxes(boxes, scores, conf_threshold, iou_threshold)
        if len(indices) > 0:
            indices = indices.flatten()
            boxes = [boxes[i] for i in indices]
            scores = [scores[i] for i in indices]
            classes = [classes[i] for i in indices]
        else:
            boxes = []
            scores = []
            classes = []
    
    return boxes, classes, scores

def draw_detections(image, boxes, classes, scores, labels):
    """Draw bounding boxes and labels on image."""
    for box, cls, score in zip(boxes, classes, scores):
        x1, y1, x2, y2 = map(int, box)
        
        # Random color for each class
        color = (int(cls * 13 % 255), int(cls * 29 % 255), int(cls * 7 % 255))
        
        # Draw box
        cv2.rectangle(image, (x1, y1), (x2, y2), color, 2)
        
        # Draw label
        label = f'{labels[cls]} {score:.2f}'
        (w, h), _ = cv2.getTextSize(label, cv2.FONT_HERSHEY_SIMPLEX, 0.6, 1)
        cv2.rectangle(image, (x1, y1 - h - 5), (x1 + w, y1), color, -1)
        cv2.putText(image, label, (x1, y1 - 5), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (255, 255, 255), 1)
    
    return image

def preprocess_image(image, input_size):
    """
    Preprocess image for YOLOv5.
    
    Args:
        image: Input image (BGR)
        input_size: Target size [height, width]
    
    Returns:
        preprocessed: Preprocessed image
        ratio: Resize ratio
        pad: Padding [top, left]
    """
    # Get original size
    h, w = image.shape[:2]
    
    # Calculate resize ratio
    target_h, target_w = input_size
    ratio = min(target_h / h, target_w / w)
    
    # Resize
    new_h, new_w = int(h * ratio), int(w * ratio)
    resized = cv2.resize(image, (new_w, new_h))
    
    # Create padded image
    padded = np.full((target_h, target_w, 3), 114, dtype=np.uint8)
    
    # Calculate padding
    pad_h = (target_h - new_h) // 2
    pad_w = (target_w - new_w) // 2
    padded[pad_h:pad_h + new_h, pad_w:pad_w + new_w] = resized
    
    # Convert to RGB (RKNN expects RGB)
    padded = cv2.cvtColor(padded, cv2.COLOR_BGR2RGB)
    
    return padded, ratio, (pad_h, pad_w)

def detect_image(rknn, image, input_size, labels, conf_threshold=0.45, iou_threshold=0.45):
    """
    Run detection on a single image.
    
    Args:
        rknn: RKNNLite instance
        image: Input image (BGR)
        input_size: Model input size [height, width]
        labels: List of class labels
        conf_threshold: Confidence threshold
        iou_threshold: NMS IoU threshold
    
    Returns:
        result_image: Image with detections drawn
        detections: List of (box, class, score) tuples
    """
    # Preprocess
    preprocessed, ratio, (pad_h, pad_w) = preprocess_image(image, input_size)
    
    # Run inference
    outputs = rknn.inference(inputs=[preprocessed])
    
    # Post-process
    boxes, classes, scores = yolov5_post_process(outputs, input_size, conf_threshold, iou_threshold)
    
    # Scale boxes back to original image size
    orig_h, orig_w = image.shape[:2]
    scaled_boxes = []
    for box in boxes:
        x1 = int((box[0] - pad_w) / ratio)
        y1 = int((box[1] - pad_h) / ratio)
        x2 = int((box[2] - pad_w) / ratio)
        y2 = int((box[3] - pad_h) / ratio)
        
        # Clip to image bounds
        x1 = max(0, min(orig_w, x1))
        y1 = max(0, min(orig_h, y1))
        x2 = max(0, min(orig_w, x2))
        y2 = max(0, min(orig_h, y2))
        scaled_boxes.append([x1, y1, x2, y2])
    
    # Draw detections
    result_image = image.copy()
    result_image = draw_detections(result_image, scaled_boxes, classes, scores, labels)
    
    detections = list(zip(scaled_boxes, classes, scores))
    return result_image, detections

def main():
    import argparse
    
    parser = argparse.ArgumentParser(description='YOLO Object Detection with RKNN')
    parser.add_argument('--model', type=str, required=True, help='Path to RKNN model')
    parser.add_argument('--image', type=str, help='Path to single image')
    parser.add_argument('--dir', type=str, help='Path to image directory')
    parser.add_argument('--camera', type=int, help='Camera device index')
    parser.add_argument('--conf', type=float, default=0.45, help='Confidence threshold')
    parser.add_argument('--iou', type=float, default=0.45, help='NMS IoU threshold')
    parser.add_argument('--output', type=str, default='results', help='Output directory')
    
    args = parser.parse_args()
    
    # Validate arguments
    if not args.image and not args.dir and args.camera is None:
        parser.error('Must specify --image, --dir, or --camera')
    
    # Load labels
    labels = load_labels(LABELS_PATH)
    print(f'Loaded {len(labels)} labels')
    
    # Initialize RKNN
    print(f'Loading model: {args.model}')
    rknn = RKNNLite()
    
    ret = rknn.load_rknn(args.model)
    if ret != 0:
        print('Failed to load RKNN model!')
        sys.exit(1)
    
    # Initialize runtime
    print('Initializing runtime...')
    ret = rknn.init_runtime()
    if ret != 0:
        print('Failed to initialize runtime!')
        sys.exit(1)
    
    # Get input shape
    input_size = [640, 640]  # Default YOLOv5 input size
    print(f'Input size: {input_size}')
    
    # Create output directory
    os.makedirs(args.output, exist_ok=True)
    
    # Process images
    if args.image:
        # Single image
        image = cv2.imread(args.image)
        if image is None:
            print(f'Failed to read image: {args.image}')
            sys.exit(1)
        
        result, detections = detect_image(rknn, image, input_size, labels, args.conf, args.iou)
        
        # Save result
        filename = os.path.basename(args.image)
        output_path = os.path.join(args.output, f'det_{filename}')
        cv2.imwrite(output_path, result)
        print(f'Detected {len(detections)} objects')
        print(f'Saved to: {output_path}')
        
    elif args.dir:
        # Directory of images
        image_extensions = ['.jpg', '.jpeg', '.png', '.bmp']
        image_files = []
        
        for f in os.listdir(args.dir):
            if os.path.splitext(f)[1].lower() in image_extensions:
                image_files.append(os.path.join(args.dir, f))
        
        print(f'Found {len(image_files)} images')
        
        for img_path in image_files:
            image = cv2.imread(img_path)
            if image is None:
                print(f'Failed to read: {img_path}')
                continue
            
            result, detections = detect_image(rknn, image, input_size, labels, args.conf, args.iou)
            
            # Save result
            filename = os.path.basename(img_path)
            output_path = os.path.join(args.output, f'det_{filename}')
            cv2.imwrite(output_path, result)
            print(f'{filename}: {len(detections)} objects detected')
    
    elif args.camera is not None:
        # Camera
        print(f'Opening camera {args.camera}...')
        cap = cv2.VideoCapture(args.camera)
        
        if not cap.isOpened():
            print('Failed to open camera!')
            sys.exit(1)
        
        print('Press "q" to quit')
        
        while True:
            ret, frame = cap.read()
            if not ret:
                print('Failed to read frame')
                break
            
            result, detections = detect_image(rknn, frame, input_size, labels, args.conf, args.iou)
            
            # Show FPS
            cv2.putText(result, f'Objects: {len(detections)}', (10, 30), 
                       cv2.FONT_HERSHEY_SIMPLEX, 1, (0, 255, 0), 2)
            
            cv2.imshow('YOLO Detection', result)
            
            if cv2.waitKey(1) & 0xFF == ord('q'):
                break
        
        cap.release()
        cv2.destroyAllWindows()
    
    # Cleanup
    rknn.release()
    print('Done!')

if __name__ == '__main__':
    main()
